//! The launcher's window: the update's progress, the two games (found, or picked with the
//! system's own pickers), the disc's conversion, and Play. The slow work (the update, the search,
//! the conversion) runs on threads that write into [`Shared`]; the window reads it every frame.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::config::{self, Config};
use crate::install::{CONVERTER, GAME, LAUNCHER};
use crate::update;

const ORANGE: egui::Color32 = egui::Color32::from_rgb(0xff, 0x8a, 0x1f);
const GOOD: egui::Color32 = egui::Color32::from_rgb(0x63, 0xd5, 0x8a);
const BAD: egui::Color32 = egui::Color32::from_rgb(0xff, 0x6b, 0x6b);
const DIM: egui::Color32 = egui::Color32::from_rgb(0xa3, 0x9c, 0xb4);

#[derive(Default)]
struct Shared {
    /// What the update is doing, and its bytes done of all.
    status: String,
    progress: Option<(u64, u64)>,
    /// The update is over (done, failed or no host): the game may start.
    updated: bool,
    update_error: Option<String>,
    /// The search for games is over, and what it found.
    searched: bool,
    found_wow: Option<PathBuf>,
    found_ctr: Option<PathBuf>,
    /// The disc is being converted; the last conversion's failure.
    converting: bool,
    convert_error: Option<String>,
    /// The kart data's converter changed and the disc is known: convert again.
    reconvert: bool,
    /// The converter's hash the update host has, for the kart data's stamp.
    converter: Option<String>,
    /// A new launcher started: this one closes.
    restart: bool,
}

struct App {
    /// The game's files (`install::root`) and the player's own (`install::state`).
    root: PathBuf,
    state: PathBuf,
    config: Config,
    shared: Arc<Mutex<Shared>>,
    started: bool,
    /// The Protons installed, newest first (Linux).
    #[cfg(target_os = "linux")]
    protons: Vec<crate::proton::Proton>,
}

/// The window's icon: the WoK badge (`dist/icon.png`), as the Windows binary's resource icon.
fn icon() -> egui::IconData {
    let decoded = image::load_from_memory_with_format(
        include_bytes!("../../../dist/icon.png"),
        image::ImageFormat::Png,
    )
    .map(|i| i.into_rgba8());
    match decoded {
        Ok(rgba) => egui::IconData {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.into_raw(),
        },
        Err(_) => egui::IconData::default(),
    }
}

pub fn run() -> eframe::Result<()> {
    let root = crate::install::root();
    let state = crate::install::state();
    let config = Config::load(&root, &state);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("World of Kartcraft")
            // Linux's Proton row needs the room.
            .with_inner_size([
                560.0,
                if cfg!(target_os = "linux") {
                    500.0
                } else {
                    420.0
                },
            ])
            .with_resizable(false)
            .with_icon(icon()),
        ..Default::default()
    };
    eframe::run_native(
        "World of Kartcraft",
        options,
        Box::new(move |cc| {
            // Dark whatever the system is set to: the game and the website are.
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            Ok(Box::new(App::new(root, state, config)))
        }),
    )
}

impl App {
    fn new(root: PathBuf, state: PathBuf, config: Config) -> App {
        let app = App {
            root,
            state,
            config,
            shared: Arc::new(Mutex::new(Shared {
                status: "Checking for updates…".into(),
                ..Default::default()
            })),
            started: false,
            #[cfg(target_os = "linux")]
            protons: crate::proton::installed(),
        };
        app.start_update();
        app.start_search();
        app
    }

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The update, on a thread: the launcher first (a new one restarts in its place), then the
    /// game's files.
    fn start_update(&self) {
        let shared = self.shared.clone();
        let root = self.root.clone();
        let state = self.state.clone();
        // The launcher's own update, where it has a manifest apart from the game's (Linux).
        #[cfg(target_os = "linux")]
        let own_base = self.config.launcher_updates();
        #[cfg(not(target_os = "linux"))]
        let own_base: Option<String> = None;
        let Some(base) = self.config.updates() else {
            let mut s = self.shared();
            s.updated = true;
            s.status = "No update server is set.".into();
            return;
        };
        std::thread::spawn(move || {
            let set =
                |f: &dyn Fn(&mut Shared)| f(&mut shared.lock().unwrap_or_else(|e| e.into_inner()));
            let manifest = match update::manifest(&base) {
                Ok(m) => m,
                Err(e) => {
                    set(&|s| {
                        s.updated = true;
                        s.status =
                            "The update server can't be reached; starting the version you have."
                                .into();
                        s.update_error = Some(e.clone());
                    });
                    return;
                }
            };
            let stale = update::stale(&root, &manifest);
            let (launcher, rest): (Vec<_>, Vec<_>) =
                stale.into_iter().partition(|f| f.path == LAUNCHER);
            // A manifest of the launcher's own, if it has one: none published yet is no update.
            let (launcher_base, launcher) = match &own_base {
                Some(own) => match update::manifest(own) {
                    Ok(m) => (own.clone(), update::stale(&root, &m)),
                    Err(_) => (own.clone(), Vec::new()),
                },
                None => (base.clone(), launcher),
            };
            let launcher: Vec<_> = launcher
                .into_iter()
                .filter(|f| f.path == LAUNCHER)
                .collect();
            // The launcher first: a new one replaces this file and starts in its place.
            if !launcher.is_empty() {
                set(&|s| s.status = "Updating the launcher…".into());
                if update::download(&launcher_base, &root, &launcher, |_, _| {}).is_ok()
                    && update::swap_in()
                {
                    set(&|s| s.restart = true);
                    return;
                }
            }
            let total: u64 = rest.iter().map(|f| f.gz_size).sum();
            if !rest.is_empty() {
                let first = !root.join(GAME).is_file();
                set(&|s| {
                    s.status = if first {
                        "Downloading the game…".into()
                    } else {
                        format!("Updating to {}…", manifest.version)
                    };
                    s.progress = Some((0, total));
                });
                let result = update::download(&base, &root, &rest, |done, _| {
                    shared.lock().unwrap_or_else(|e| e.into_inner()).progress = Some((done, total));
                });
                if let Err(e) = result {
                    set(&|s| {
                        s.updated = true;
                        s.progress = None;
                        s.status = format!("The update failed: {e}");
                        s.update_error = Some(e.clone());
                    });
                    return;
                }
            }
            update::write_version(&state, &manifest);
            // Kart data from before the launcher kept the stamp is taken as current.
            let stamp = state.join("kart-data").join(".converter");
            if !stamp.is_file() && state.join("kart-data").join("kart.json").is_file() {
                if let Some(c) = &manifest.converter {
                    let _ = std::fs::write(&stamp, c);
                }
            }
            let reconvert = update::converter_changed(&state, &manifest);
            set(&|s| {
                s.updated = true;
                s.progress = None;
                s.status = format!("Up to date ({}).", manifest.version);
                s.converter = manifest.converter.clone();
                s.reconvert = reconvert;
            });
        });
    }

    /// The search for the games the settings don't name yet, on a thread.
    fn start_search(&self) {
        let need_wow = self
            .config
            .wow_dir
            .as_deref()
            .and_then(config::wow_folder)
            .is_none();
        let need_ctr = !self.converted()
            && self
                .config
                .ctr_disc
                .as_deref()
                .and_then(config::disc_image)
                .is_none();
        let shared = self.shared.clone();
        let root = self.root.clone();
        std::thread::spawn(move || {
            let wow = need_wow.then(|| crate::find::wow(&root)).flatten();
            let ctr = need_ctr.then(|| crate::find::ctr(&root)).flatten();
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.found_wow = wow;
            s.found_ctr = ctr;
            s.searched = true;
        });
    }

    fn converted(&self) -> bool {
        self.state.join("kart-data").join("kart.json").is_file()
    }

    /// The disc into `kart-data`, on a thread, by the converter the update brought.
    fn convert(&self, disc: PathBuf) {
        let shared = self.shared.clone();
        let root = self.root.clone();
        let state = self.state.clone();
        let config = self.config.clone();
        {
            let mut s = self.shared();
            s.converting = true;
            s.convert_error = None;
            s.reconvert = false;
        }
        std::thread::spawn(move || {
            let out = crate::launch::converter(&state, &config, &root.join(CONVERTER), &disc)
                .map_err(std::io::Error::other)
                .and_then(|mut cmd| cmd.output());
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.converting = false;
            match out {
                Ok(o) if o.status.success() => {
                    let stamp = update::sha256(&root.join(CONVERTER));
                    if let Some(stamp) = stamp {
                        let _ = std::fs::write(state.join("kart-data").join(".converter"), stamp);
                    }
                }
                Ok(o) => {
                    // The converter's own reason (another version of the game, no disc at all),
                    // else what it needs.
                    let said = String::from_utf8_lossy(&o.stderr);
                    let reason = said
                        .lines()
                        .rev()
                        .find_map(|l| l.strip_prefix("ctr-extract: "))
                        .map(str::to_string);
                    s.convert_error = Some(reason.unwrap_or_else(|| {
                        "That isn't the disc the game needs: it has to be the US Crash Team \
                         Racing (SCUS-94426), as a .bin with its .cue, or an .iso."
                            .into()
                    }));
                }
                Err(e) => s.convert_error = Some(format!("The converter didn't start: {e}")),
            }
        });
    }

    fn wow(&self) -> Option<PathBuf> {
        self.config.wow_dir.as_deref().and_then(config::wow_folder)
    }

    fn disc(&self) -> Option<PathBuf> {
        self.config.ctr_disc.as_deref().and_then(config::disc_image)
    }

    fn play(&mut self, ctx: &egui::Context) {
        let Some(wow) = self.wow() else { return };
        self.config.account_seen = true;
        self.config.save(&self.state);
        let started = crate::launch::game(&self.state, &self.config, &self.root.join(GAME), &wow)
            .map_err(std::io::Error::other)
            .and_then(|mut cmd| cmd.spawn());
        match started {
            Ok(_) => {
                self.started = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(e) => self.shared().update_error = Some(format!("The game didn't start: {e}")),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // What the threads found, into the settings.
        let (found_wow, found_ctr, restart) = {
            let mut s = self.shared();
            (s.found_wow.take(), s.found_ctr.take(), s.restart)
        };
        if restart {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let mut changed = false;
        if self.wow().is_none() {
            if let Some(w) = found_wow {
                self.config.wow_dir = Some(w);
                changed = true;
            }
        }
        if self.disc().is_none() && !self.converted() {
            if let Some(c) = found_ctr {
                self.config.ctr_disc = Some(c);
                changed = true;
            }
        }
        if changed {
            self.config.save(&self.state);
        }
        // The disc converts once the converter is here: the first time, or when it changed.
        let (updated, converting, reconvert) = {
            let s = self.shared();
            (s.updated, s.converting, s.reconvert)
        };
        let converter_here = self.root.join(CONVERTER).is_file();
        let failed = self.shared().convert_error.is_some();
        if updated && converter_here && !converting && !failed {
            if let Some(disc) = self.disc() {
                if !self.converted() || reconvert {
                    self.convert(disc);
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("World of Kartcraft").size(28.0).strong().color(ORANGE));
            });
            ui.add_space(10.0);

            // The update.
            let (status, progress, update_error, searched, converting, convert_error) = {
                let s = self.shared();
                (
                    s.status.clone(),
                    s.progress,
                    s.update_error.clone(),
                    s.searched,
                    s.converting,
                    s.convert_error.clone(),
                )
            };
            ui.label(egui::RichText::new(&status).color(DIM));
            if let Some((done, total)) = progress {
                let f = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                ui.add(
                    egui::ProgressBar::new(f.clamp(0.0, 1.0))
                        .text(format!("{:.1} of {:.1} MB", done as f64 / 1e6, total as f64 / 1e6)),
                );
            }
            ui.add_space(12.0);

            // The two games.
            let wow = self.wow();
            game_row(
                ui,
                "World of Warcraft 1.12.1",
                wow.as_ref().map(|p| p.display().to_string()),
                if searched { "Not found. Pick the folder that has Data in it." } else { "Looking for it…" },
                || {
                    rfd::FileDialog::new()
                        .set_title("Your World of Warcraft 1.12.1 folder (the one with Data in it)")
                        .pick_folder()
                },
                |picked| match config::wow_folder(&picked) {
                    Some(dir) => {
                        self.config.wow_dir = Some(dir);
                        self.config.save(&self.state);
                        None
                    }
                    None => Some(format!(
                        "That folder has no Data{sep}dbc.MPQ: it needs to be an English 1.12.1 client.",
                        sep = std::path::MAIN_SEPARATOR
                    )),
                },
            );
            ui.add_space(6.0);
            let ctr_text = if converting {
                Some("Converting the kart, sounds and music from your disc…".to_string())
            } else if self.converted() {
                Some(match self.disc() {
                    Some(d) => format!("Ready, from {}", d.display()),
                    None => "Ready".to_string(),
                })
            } else {
                None
            };
            let state = self.state.clone();
            let shared = self.shared.clone();
            game_row(
                ui,
                "Crash Team Racing (US disc image)",
                ctr_text,
                if !searched {
                    "Looking for it…"
                } else if self.disc().is_some() {
                    "Waiting for the converter to download…"
                } else {
                    "Not found. Pick your .bin, .cue or .iso."
                },
                || {
                    rfd::FileDialog::new()
                        .set_title("Your Crash Team Racing disc image (US, SCUS-94426)")
                        .add_filter("Disc image", &["bin", "cue", "iso", "img"])
                        .pick_file()
                },
                |picked| match config::disc_image(&picked) {
                    Some(disc) => {
                        self.config.ctr_disc = Some(disc);
                        self.config.save(&state);
                        // A new disc converts afresh.
                        let _ = std::fs::remove_file(state.join("kart-data").join("kart.json"));
                        shared.lock().unwrap_or_else(|e| e.into_inner()).convert_error = None;
                        None
                    }
                    None => Some("Pick the disc image itself: a .bin (with its .cue), a .cue or an .iso.".into()),
                },
            );
            #[cfg(target_os = "linux")]
            {
                ui.add_space(6.0);
                self.proton_row(ui);
            }
            if let Some(e) = &convert_error {
                ui.label(egui::RichText::new(e).color(BAD));
            }
            if let Some(e) = &update_error {
                ui.label(egui::RichText::new(format!("({e})")).color(DIM).small());
            }

            ui.add_space(16.0);
            let ready = wow.is_some()
                && crate::launch::blocked(&self.config).is_none()
                && self.converted()
                && !converting
                && self.root.join(GAME).is_file()
                && (updated || self.root.join(GAME).is_file() && update_error.is_some());
            ui.vertical_centered(|ui| {
                let button = egui::Button::new(egui::RichText::new("PLAY").size(24.0).strong().color(egui::Color32::BLACK))
                    .fill(if ready { ORANGE } else { egui::Color32::from_gray(70) })
                    .min_size(egui::vec2(220.0, 48.0));
                if ui.add_enabled(ready, button).clicked() {
                    self.play(ctx);
                }
                ui.add_space(10.0);
                let site = self
                    .config
                    .register_url
                    .clone()
                    .unwrap_or_else(|| "https://worldofkartcraft.com".into());
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 300.0).max(0.0) / 2.0);
                    if !self.config.account_seen {
                        ui.hyperlink_to(egui::RichText::new("Make an account").color(ORANGE), format!("{site}/signup"));
                    } else {
                        ui.hyperlink_to("Account sign-up", format!("{site}/signup"));
                    }
                    ui.label("·");
                    ui.hyperlink_to("Help", format!("{site}/help/"));
                    ui.label("·");
                    ui.hyperlink_to("Discord", format!("{site}/discord"));
                });
            });
        });
        if !self.started {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

#[cfg(target_os = "linux")]
impl App {
    /// The Proton row: the one the game runs with, the others installed to pick from, Browse… for
    /// one elsewhere, and a warning when the pick is older than the Proton that made the prefix.
    fn proton_row(&mut self, ui: &mut egui::Ui) {
        let chosen = crate::launch::proton(&self.config);
        let name_of = |script: &PathBuf| {
            self.protons
                .iter()
                .find(|p| &p.script == script)
                .map(|p| p.name.clone())
                .or_else(|| crate::proton::at(script.parent()?).map(|p| p.name))
                .unwrap_or_else(|| script.display().to_string())
        };
        let chosen_name = chosen.as_ref().map(name_of);
        let warning = crate::proton::prefix_made_by(&self.root)
            .zip(chosen_name.clone())
            .and_then(|(made, picked)| crate::proton::downgrade(&made, &picked));
        let mut picked: Option<PathBuf> = None;
        let mut error: Option<String> = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let ok = chosen.is_some();
                ui.label(
                    egui::RichText::new(if ok { "✔" } else { "✖" }).color(if ok { GOOD } else { BAD }),
                );
                ui.label(egui::RichText::new("Proton").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("A Proton's proton script")
                            .pick_file()
                        {
                            match p.parent().and_then(crate::proton::at) {
                                Some(found) => picked = Some(found.script),
                                None => error = Some("Pick the file named proton inside a Proton's folder.".into()),
                            }
                        }
                    }
                    egui::ComboBox::from_id_salt("proton")
                        .selected_text(chosen_name.clone().unwrap_or_else(|| "none".into()))
                        .show_ui(ui, |ui| {
                            for p in &self.protons {
                                let on = chosen.as_ref() == Some(&p.script);
                                if ui.selectable_label(on, &p.name).clicked() {
                                    picked = Some(p.script.clone());
                                }
                            }
                        });
                });
            });
            let text = match &chosen {
                Some(p) => p.display().to_string(),
                None => "Not found. Install one through Steam (or GE-Proton), or pick its proton script.".into(),
            };
            ui.label(egui::RichText::new(text).color(DIM).small());
            if let Some(w) = &warning {
                ui.label(egui::RichText::new(w).color(BAD).small());
            }
            if let Some(e) = &error {
                ui.label(egui::RichText::new(e).color(BAD).small());
            }
        });
        if let Some(p) = picked {
            self.config.proton = Some(p);
            self.config.save(&self.state);
        }
    }
}

/// One game's row: its name, where it is (green) or what is missing (red), and Browse….
fn game_row(
    ui: &mut egui::Ui,
    title: &str,
    found: Option<String>,
    missing: &str,
    pick: impl FnOnce() -> Option<PathBuf>,
    mut accept: impl FnMut(PathBuf) -> Option<String>,
) {
    let id = ui.make_persistent_id(title);
    let mut error: Option<String> = ui.data(|d| d.get_temp(id));
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let ok = found.is_some();
            ui.label(
                egui::RichText::new(if ok { "✔" } else { "✖" }).color(if ok { GOOD } else { BAD }),
            );
            ui.label(egui::RichText::new(title).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Browse…").clicked() {
                    if let Some(p) = pick() {
                        error = accept(p);
                    }
                }
            });
        });
        match &found {
            Some(text) => ui.label(egui::RichText::new(text).color(DIM).small()),
            None => ui.label(egui::RichText::new(missing).color(DIM).small()),
        };
        if let Some(e) = &error {
            ui.label(egui::RichText::new(e).color(BAD).small());
        }
    });
    ui.data_mut(|d| d.insert_temp(id, error));
}
