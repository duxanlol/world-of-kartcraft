//! The HID side on Windows: a thread that finds Sony pads, opens them, reads their input reports
//! into [`Shared`] and writes the rumble asked for. A Bluetooth pad is switched to its full report
//! at once (a first, still output report does it, as it does for SDL and Steam), so it reads the
//! same whether or not anything has rumbled yet.

use std::collections::HashMap;
use std::ffi::CString;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hidapi::{BusType, HidApi, HidDevice};

use super::report::{self, Model, PadState};

/// One pad as the thread last read it.
#[derive(Clone, Debug)]
pub struct Slot {
    pub model: Model,
    pub product: u16,
    /// `None` until its first report.
    pub state: Option<PadState>,
}

/// What the thread shares with the game: each open pad by its own number, gone when it is.
#[derive(Default)]
pub struct Shared {
    pub pads: HashMap<u64, Slot>,
}

/// The game's end: the pads read, and the motors' strengths to set.
pub struct Link {
    pub shared: Arc<Mutex<Shared>>,
    pub motors: Sender<(u8, u8)>,
}

/// How often the thread looks for pads newly plugged in or paired.
const SCAN: Duration = Duration::from_secs(2);
/// The thread's rest between reads of every pad: 250 Hz, the pads' own Bluetooth rate.
const REST: Duration = Duration::from_millis(4);

struct Open {
    id: u64,
    device: HidDevice,
    model: Model,
    bluetooth: bool,
    seq: u8,
}

impl Open {
    fn rumble(&mut self, (strong, weak): (u8, u8)) -> bool {
        let r = report::rumble(self.model, self.bluetooth, self.seq, strong, weak);
        self.seq = self.seq.wrapping_add(1);
        self.device.write(&r).is_ok()
    }
}

/// Starts the thread; `None` when HID cannot be opened at all.
pub fn start() -> Option<Link> {
    let api = match HidApi::new() {
        Ok(api) => api,
        Err(e) => {
            bevy::log::warn!("kart: PlayStation pads over HID unavailable: {e}");
            return None;
        }
    };
    let shared = Arc::new(Mutex::new(Shared::default()));
    let (motors, rx) = std::sync::mpsc::channel();
    let thread_shared = shared.clone();
    std::thread::Builder::new()
        .name("sony-pads".into())
        .spawn(move || run(api, thread_shared, rx))
        .ok()?;
    Some(Link { shared, motors })
}

fn run(mut api: HidApi, shared: Arc<Mutex<Shared>>, motors: Receiver<(u8, u8)>) {
    let mut open: Vec<Open> = Vec::new();
    let mut paths: HashMap<u64, CString> = HashMap::new();
    let mut next_id = 1u64;
    let mut scanned: Option<Instant> = None;
    let mut want = (0u8, 0u8);
    let mut buf = [0u8; 128];
    loop {
        if scanned.is_none_or(|t| t.elapsed() >= SCAN) {
            scanned = Some(Instant::now());
            if api.refresh_devices().is_ok() {
                let found: Vec<_> = api
                    .device_list()
                    .filter(|d| d.vendor_id() == report::SONY)
                    .filter(|d| d.usage_page() == 0x01 && d.usage() == 0x05)
                    .filter_map(|d| {
                        Some((
                            d.path().to_owned(),
                            Model::of(d.product_id())?,
                            d.product_id(),
                            d.bus_type(),
                        ))
                    })
                    .collect();
                for (path, model, product, bus) in found {
                    if paths.values().any(|p| *p == path) {
                        continue;
                    }
                    let Ok(device) = api.open_path(&path) else {
                        continue;
                    };
                    let _ = device.set_blocking_mode(false);
                    let bluetooth = matches!(bus, BusType::Bluetooth);
                    let id = next_id;
                    next_id += 1;
                    let mut pad = Open {
                        id,
                        device,
                        model,
                        bluetooth,
                        seq: 0,
                    };
                    // Over Bluetooth the first output report switches the pad to its full input
                    // report; its motors start as the game wants them. A paired pad that is off
                    // can still be listed: one that takes no report is not there.
                    if !pad.rumble(want) {
                        continue;
                    }
                    bevy::log::info!(
                        "kart: {} over {} read directly (HID)",
                        model.name(),
                        if bluetooth { "Bluetooth" } else { "USB" }
                    );
                    paths.insert(id, path);
                    shared
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .pads
                        .insert(
                            id,
                            Slot {
                                model,
                                product,
                                state: None,
                            },
                        );
                    open.push(pad);
                }
            }
        }

        // The newest strengths asked for since the last pass.
        let mut changed = false;
        loop {
            match motors.try_recv() {
                Ok(m) => {
                    changed |= m != want;
                    want = m;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            }
        }

        let mut lost = Vec::new();
        for pad in &mut open {
            if changed && !pad.rumble(want) {
                lost.push(pad.id);
                continue;
            }
            let mut latest = None;
            loop {
                match pad.device.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Some(s) = report::parse(pad.model, &buf[..n]) {
                            latest = Some(s);
                        }
                    }
                    Err(_) => {
                        lost.push(pad.id);
                        break;
                    }
                }
            }
            if let Some(s) = latest {
                if let Some(slot) = shared
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .pads
                    .get_mut(&pad.id)
                {
                    slot.state = Some(s);
                }
            }
        }
        if !lost.is_empty() {
            open.retain(|p| !lost.contains(&p.id));
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            for id in lost {
                paths.remove(&id);
                s.pads.remove(&id);
                bevy::log::info!("kart: a PlayStation pad read directly is gone");
            }
        }
        std::thread::sleep(REST);
    }
}
