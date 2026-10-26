//! Sony's controllers (DualSense, DualShock 4) read over HID on Windows, for when gilrs's
//! Windows.Gaming.Input sees none: a machine whose GameInput redistributable has taken WGI's pads
//! away still shows them to joy.cpl and HID. A thread reads the pad's input reports and this turns
//! them into the raw gamepad events `bevy_gilrs` writes, as `android.rs` does for a phone's pads;
//! the pad is ours only while gilrs has no Sony pad of its own, so it is never there twice.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

use bevy::input::gamepad::{
    Gamepad, GamepadAxis, GamepadButton, GamepadConnection, GamepadConnectionEvent,
    RawGamepadAxisChangedEvent, RawGamepadButtonChangedEvent, RawGamepadEvent,
};
use bevy::input::InputSystems;
use bevy::prelude::*;

/// Sony's USB vendor id.
const SONY: u16 = 0x054c;
/// The DualSense, the DualSense Edge, and the two DualShock 4s.
const DUALSENSE: [u16; 2] = [0x0ce6, 0x0df2];
const DUALSHOCK4: [u16; 2] = [0x05c4, 0x09cc];

/// The buttons a report carries, in [`Pad::buttons`]'s order.
const BUTTONS: [GamepadButton; 17] = [
    GamepadButton::West,
    GamepadButton::South,
    GamepadButton::East,
    GamepadButton::North,
    GamepadButton::LeftTrigger,
    GamepadButton::RightTrigger,
    GamepadButton::LeftTrigger2,
    GamepadButton::RightTrigger2,
    GamepadButton::Select,
    GamepadButton::Start,
    GamepadButton::LeftThumb,
    GamepadButton::RightThumb,
    GamepadButton::Mode,
    GamepadButton::DPadUp,
    GamepadButton::DPadDown,
    GamepadButton::DPadLeft,
    GamepadButton::DPadRight,
];
const AXES: [GamepadAxis; 4] = [
    GamepadAxis::LeftStickX,
    GamepadAxis::LeftStickY,
    GamepadAxis::RightStickX,
    GamepadAxis::RightStickY,
];

/// One input report: each of [`BUTTONS`] (the triggers analog, 0..1) and of [`AXES`] (-1..1, up
/// positive as gilrs's).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Pad {
    buttons: [f32; BUTTONS.len()],
    axes: [f32; AXES.len()],
}

/// Which pad, by gilrs's own name for it (`kart::pad_choice` keys a pad on name and ids).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    DualSense(u16),
    DualShock4(u16),
}

impl Kind {
    fn of(product: u16) -> Option<Kind> {
        if DUALSENSE.contains(&product) {
            Some(Kind::DualSense(product))
        } else if DUALSHOCK4.contains(&product) {
            Some(Kind::DualShock4(product))
        } else {
            None
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::DualSense(_) => "PS5 Controller",
            Kind::DualShock4(_) => "PS4 Controller",
        }
    }

    fn product(self) -> u16 {
        match self {
            Kind::DualSense(p) | Kind::DualShock4(p) => p,
        }
    }
}

enum Msg {
    Connected(Kind),
    Report(Pad),
    Gone,
}

/// A stick byte (0..255, 128 centred) as an axis.
fn stick(b: u8) -> f32 {
    ((f32::from(b) - 128.0) / 127.0).clamp(-1.0, 1.0)
}

/// `report` (its id first) as a [`Pad`]. Two layouts at an offset: the full one of a DualSense's
/// USB report 0x01 and its Bluetooth 0x31, and the short one of a DualSense's Bluetooth 0x01 (sent
/// until the host asks for more) and a DualShock 4's USB 0x01 and Bluetooth 0x11.
fn parse(kind: Kind, bluetooth: bool, report: &[u8]) -> Option<Pad> {
    let ds5 = matches!(kind, Kind::DualSense(_));
    // (sticks, triggers, face and D-pad) offsets.
    let (s, t, f) = match report.first()? {
        0x01 if ds5 && !bluetooth => (1, 5, 8),
        0x31 if ds5 => (2, 6, 9),
        0x01 => (1, 8, 5),
        0x11 if !ds5 => (3, 10, 7),
        _ => return None,
    };
    let at = |i: usize| report.get(i).copied();
    let (face, more, system) = (at(f)?, at(f + 1)?, at(f + 2)?);
    let bit = |b: u8, n: u32| f32::from(u8::from(b & (1 << n) != 0));
    let mut pad = Pad::default();
    // Square, cross, circle, triangle.
    for i in 0..4 {
        pad.buttons[i] = bit(face, 4 + i as u32);
    }
    // L1, R1, then the triggers' own analog travel, then create, options, L3, R3.
    pad.buttons[4] = bit(more, 0);
    pad.buttons[5] = bit(more, 1);
    pad.buttons[6] = f32::from(at(t)?) / 255.0;
    pad.buttons[7] = f32::from(at(t + 1)?) / 255.0;
    for i in 0..4 {
        pad.buttons[8 + i] = bit(more, 4 + i as u32);
    }
    pad.buttons[12] = bit(system, 0);
    // The hat: 0 up, clockwise in eighths, 8 and above none.
    let hat = face & 0x0f;
    let on = |dirs: [u8; 3]| f32::from(u8::from(dirs.contains(&hat)));
    pad.buttons[13] = on([7, 0, 1]);
    pad.buttons[14] = on([3, 4, 5]);
    pad.buttons[15] = on([5, 6, 7]);
    pad.buttons[16] = on([1, 2, 3]);
    pad.axes = [
        stick(at(s)?),
        -stick(at(s + 1)?),
        stick(at(s + 2)?),
        -stick(at(s + 3)?),
    ];
    Some(pad)
}

/// Opens the first Sony pad there is and reads it until it goes, then looks again.
fn read(tx: Sender<Msg>) {
    let Ok(mut api) = hidapi::HidApi::new() else {
        return;
    };
    loop {
        let _ = api.refresh_devices();
        let found = api.device_list().find_map(|d| {
            let kind = (d.vendor_id() == SONY && d.usage_page() == 1 && matches!(d.usage(), 4 | 5))
                .then(|| Kind::of(d.product_id()))
                .flatten()?;
            Some((d.clone(), kind))
        });
        let opened = found.and_then(|(info, kind)| {
            let dev = info.open_device(&api).ok()?;
            Some((
                dev,
                kind,
                matches!(info.bus_type(), hidapi::BusType::Bluetooth),
            ))
        });
        let Some((dev, kind, bluetooth)) = opened else {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        info!("{} read over HID", kind.name());
        if tx.send(Msg::Connected(kind)).is_err() {
            return;
        }
        let mut buf = [0u8; 128];
        loop {
            match dev.read_timeout(&mut buf, 500) {
                Ok(0) => {}
                Ok(n) => {
                    if let Some(pad) = parse(kind, bluetooth, &buf[..n]) {
                        if tx.send(Msg::Report(pad)).is_err() {
                            return;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        if tx.send(Msg::Gone).is_err() {
            return;
        }
    }
}

#[derive(Resource)]
struct SonyPads {
    rx: Mutex<Receiver<Msg>>,
    /// The pad read over HID, and its latest report.
    device: Option<(Kind, Pad)>,
    /// The gamepad entity it is while connected, and what it was last sent as.
    pad: Option<(Entity, Pad)>,
    /// The entity, kept for the next connection.
    entity: Option<Entity>,
}

pub struct SonyPadPlugin;

impl Plugin for SonyPadPlugin {
    fn build(&self, app: &mut App) {
        let (tx, rx) = channel();
        if std::thread::Builder::new()
            .name("sony pad".into())
            .spawn(move || read(tx))
            .is_err()
        {
            return;
        }
        app.insert_resource(SonyPads {
            rx: Mutex::new(rx),
            device: None,
            pad: None,
            entity: None,
        })
        .add_systems(PreUpdate, feed.before(InputSystems));
    }
}

fn feed(
    mut commands: Commands,
    mut sony: ResMut<SonyPads>,
    gamepads: Query<(Entity, &Gamepad)>,
    mut events: MessageWriter<RawGamepadEvent>,
    mut connections: MessageWriter<GamepadConnectionEvent>,
    mut buttons: MessageWriter<RawGamepadButtonChangedEvent>,
    mut axes: MessageWriter<RawGamepadAxisChangedEvent>,
) {
    let sony = &mut *sony;
    for msg in sony
        .rx
        .get_mut()
        .map(|rx| rx.try_iter().collect::<Vec<_>>())
        .unwrap_or_default()
    {
        match msg {
            Msg::Connected(kind) => sony.device = Some((kind, Pad::default())),
            Msg::Report(pad) => {
                if let Some((_, last)) = &mut sony.device {
                    *last = pad;
                }
            }
            Msg::Gone => sony.device = None,
        }
    }
    let ours = sony.pad.map(|(e, _)| e);
    let gilrs_has_one = gamepads
        .iter()
        .any(|(e, g)| Some(e) != ours && g.vendor_id() == Some(SONY));
    let want = sony.device.filter(|_| !gilrs_has_one);
    let mut connection = |pad: Entity, connection| {
        let event = GamepadConnectionEvent::new(pad, connection);
        events.write(event.clone().into());
        connections.write(event);
    };
    match (want, sony.pad) {
        (None, Some((pad, _))) => {
            connection(pad, GamepadConnection::Disconnected);
            sony.pad = None;
        }
        (Some((kind, _)), None) => {
            let pad = *sony
                .entity
                .get_or_insert_with(|| commands.spawn_empty().id());
            connection(
                pad,
                GamepadConnection::Connected {
                    name: kind.name().to_string(),
                    vendor_id: Some(SONY),
                    product_id: Some(kind.product()),
                },
            );
            // Every value is sent the first time: the last sent is one nothing equals.
            let unsent = Pad {
                buttons: [f32::NAN; BUTTONS.len()],
                axes: [f32::NAN; AXES.len()],
            };
            sony.pad = Some((pad, unsent));
        }
        _ => {}
    }
    let (Some((_, now)), Some((pad, sent))) = (want, sony.pad.as_mut()) else {
        return;
    };
    for (i, button) in BUTTONS.into_iter().enumerate() {
        if now.buttons[i] != sent.buttons[i] {
            let event = RawGamepadButtonChangedEvent::new(*pad, button, now.buttons[i]);
            events.write(event.into());
            buttons.write(event);
        }
    }
    for (i, axis) in AXES.into_iter().enumerate() {
        if now.axes[i] != sent.axes[i] {
            let event = RawGamepadAxisChangedEvent::new(*pad, axis, now.axes[i]);
            events.write(event.into());
            axes.write(event);
        }
    }
    *sent = now;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dualsense_over_bluetooth_short_report() {
        // As read off a DualSense over Bluetooth: sticks near centre, nothing held.
        let mut r = vec![0x01, 0x83, 0x7d, 0x81, 0x7f, 0x08, 0x00, 0x20, 0x00, 0x00];
        let pad = parse(Kind::DualSense(0x0ce6), true, &r).unwrap();
        assert!(pad.buttons.iter().all(|&b| b == 0.0));
        assert!(pad.axes.iter().all(|a| a.abs() < 0.05));
        // Cross, the hat right, R1, R2 half down, the left stick full up.
        r[5] = 0x20 | 2;
        r[6] = 0x02;
        r[9] = 128;
        r[2] = 0;
        let pad = parse(Kind::DualSense(0x0ce6), true, &r).unwrap();
        assert_eq!(pad.buttons[1], 1.0, "cross");
        assert_eq!(pad.buttons[16], 1.0, "right");
        assert_eq!(pad.buttons[13], 0.0, "not up");
        assert_eq!(pad.buttons[5], 1.0, "R1");
        assert!((pad.buttons[7] - 0.5).abs() < 0.01, "R2");
        assert_eq!(pad.axes[1], 1.0, "up is positive");
    }

    #[test]
    fn a_dualsense_full_report_over_usb_and_bluetooth() {
        // USB 0x01: sticks at 1.., triggers at 5, 6, face at 8.
        let mut usb = vec![0u8; 64];
        usb[0] = 0x01;
        usb[1..5].copy_from_slice(&[128, 128, 255, 128]);
        usb[5] = 255;
        usb[8] = 0x80 | 8;
        usb[10] = 0x01;
        let pad = parse(Kind::DualSense(0x0ce6), false, &usb).unwrap();
        assert_eq!(pad.buttons[3], 1.0, "triangle");
        assert_eq!(pad.buttons[6], 1.0, "L2");
        assert_eq!(pad.buttons[12], 1.0, "PS");
        assert_eq!(pad.axes[2], 1.0, "right stick right");
        assert!(
            pad.buttons[13..].iter().all(|&b| b == 0.0),
            "the hat at rest"
        );
        // Bluetooth 0x31: the same one byte on.
        let mut bt = vec![0x31];
        bt.extend_from_slice(&usb);
        bt[1] = 0;
        let same = parse(Kind::DualSense(0x0ce6), true, &bt).unwrap();
        assert_eq!(same, pad);
    }

    #[test]
    fn a_dualshock4_and_an_unknown_report() {
        let mut usb = vec![0u8; 64];
        usb[0] = 0x01;
        usb[1..5].copy_from_slice(&[128; 4]);
        usb[5] = 0x10 | 8;
        let pad = parse(Kind::DualShock4(0x09cc), false, &usb).unwrap();
        assert_eq!(pad.buttons[0], 1.0, "square");
        assert!(parse(Kind::DualShock4(0x09cc), true, &[0x05, 0, 0]).is_none());
        assert!(
            parse(Kind::DualSense(0x0ce6), true, &[0x01, 1]).is_none(),
            "short"
        );
    }
}
