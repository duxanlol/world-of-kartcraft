//! Kartcraft's settings as CVars, the channel the layer's settings window writes
//! (`assets/ui/KartcraftFrames.xml`, `/kartcraft`): `SetCVar` there, an observer here, and the
//! registry persists them in `config.toml` as it does every other row (`crate::cvars`).
//!
//! - `KartArrowShow`, `KartArrowSize`, `KartArrowX`, `KartArrowY`: the race's guide arrow
//!   ([`GuideHud`]); `KartArrowMove` is its placement mode, held for the session only.
//! - `KartChaseCamera`: CTR's chase camera, else our own follow camera.
//! - `KartMusic`, `KartMusicVolume`: CTR's race music, on top of the client's music volume.
//! - `KartClass`, `KartRacer`: the garage's engine class (as the Karting skill allows it,
//!   [`super::garage`]) and whose kart we drive; both apply at once, while driving too.
//! - `KartGhost`: the ghost of our best run on the course races us ([`super::ghost`]); the runs
//!   are recorded and kept either way.
//!
//! `KartMinimapButton` and `KartMinimapAngle` are the layer's alone.

use bevy::prelude::*;

use super::race::GuideHud;

/// The kart's own settings, as their CVars say.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub(crate) struct KartSettings {
    /// Drive with CTR's chase camera rather than our follow camera.
    pub(crate) chase_camera: bool,
    pub(crate) music: bool,
    /// The music's share of the client's music volume, 0 to 1.
    pub(crate) music_volume: f32,
    /// The engine class picked in the garage; the one driven is [`super::garage::effective`]'s.
    pub(crate) class: kartcraft::Class,
    /// Race the ghost of our best run on the course.
    pub(crate) ghost: bool,
}

impl Default for KartSettings {
    fn default() -> Self {
        Self {
            chase_camera: true,
            music: true,
            music_volume: 1.0,
            class: super::garage::TIERS[0].0,
            ghost: true,
        }
    }
}

impl KartSettings {
    /// The camera to drive with: the setting, unless `WOW_KART_CAM=wow` holds our own for the run.
    pub(crate) fn chase_camera(&self) -> bool {
        self.chase_camera && std::env::var("WOW_KART_CAM").map_or(true, |v| v != "wow")
    }

    /// The music's volume against the client's music slider.
    pub(crate) fn music_level(&self) -> f32 {
        if self.music {
            self.music_volume.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// The Kartcraft CVars' change callback.
pub(crate) fn on_cvar(
    ev: On<crate::cvars::CvarChanged>,
    mut hud: ResMut<GuideHud>,
    mut settings: ResMut<KartSettings>,
) {
    match ev.key().as_str() {
        "kartarrowshow" => hud.shown = ev.flag(),
        "kartarrowsize" => hud.scale = ev.num().clamp(GuideHud::MIN_SCALE, GuideHud::MAX_SCALE),
        "kartarrowx" => hud.x = ev.num().clamp(0.0, 1.0),
        "kartarrowy" => hud.y = ev.num().clamp(0.0, 1.0),
        "kartarrowmove" => hud.editing = ev.flag(),
        "kartchasecamera" => settings.chase_camera = ev.flag(),
        "kartmusic" => settings.music = ev.flag(),
        "kartmusicvolume" => settings.music_volume = ev.num().clamp(0.0, 1.0),
        "kartghost" => settings.ghost = ev.flag(),
        "kartclass" => {
            settings.class = kartcraft::class_named(&ev.new).unwrap_or(super::garage::TIERS[0].0)
        }
        "kartracer" if !kartcraft::model::set_our_kart(ev.new.trim()) => {
            warn!("kart: no kart for racer {:?} in the kart data", ev.new);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cvars::{CvarChanged, REGISTERED};

    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<GuideHud>()
            .init_resource::<KartSettings>()
            .add_observer(on_cvar);
        app
    }

    fn set(app: &mut App, name: &str, value: &str) {
        app.world_mut().trigger(CvarChanged {
            name: name.into(),
            old: String::new(),
            new: value.into(),
        });
    }

    #[test]
    fn the_registered_defaults_are_the_knobs_defaults() {
        let d = |name: &str| -> f32 {
            REGISTERED
                .iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("{name} is registered"))
                .default
                .parse()
                .unwrap()
        };
        let hud = GuideHud::default();
        assert_eq!(d("KartArrowX"), hud.x);
        assert_eq!(d("KartArrowY"), hud.y);
        assert_eq!(d("KartArrowSize"), hud.scale);
        assert_eq!(d("KartArrowShow") != 0.0, hud.shown);
        assert_eq!(d("KartArrowMove") != 0.0, hud.editing);
        let settings = KartSettings::default();
        assert_eq!(d("KartChaseCamera") != 0.0, settings.chase_camera);
        assert_eq!(d("KartMusic") != 0.0, settings.music);
        assert_eq!(d("KartMusicVolume"), settings.music_volume);
        assert_eq!(d("KartGhost") != 0.0, settings.ghost);
        for name in ["KartMinimapButton", "KartMinimapAngle"] {
            d(name);
        }
        let text = |name: &str| REGISTERED.iter().find(|r| r.name == name).unwrap().default;
        assert_eq!(text("KartClass"), kartcraft::class_key(settings.class));
        assert_eq!(text("KartRacer"), "crash");
    }

    #[test]
    fn the_garage_s_class_is_kept_and_the_skill_decides_what_is_driven() {
        let mut app = app();
        set(&mut app, "KartClass", "speed");
        let picked = app.world().resource::<KartSettings>().class;
        assert_eq!(picked, kartcraft::Class::Speed);
        assert_eq!(
            super::super::garage::effective(picked, Some(100)),
            kartcraft::Class::Balanced
        );
        set(&mut app, "kartclass", "Acceleration");
        assert_eq!(
            app.world().resource::<KartSettings>().class,
            kartcraft::Class::Accel
        );
        set(&mut app, "KartClass", "rocket");
        assert_eq!(
            app.world().resource::<KartSettings>().class,
            kartcraft::Class::Turn,
            "an unknown class is the starting one"
        );
        // A racer the kart data has no kart for changes nothing.
        let before = kartcraft::model::our_kart();
        set(&mut app, "KartRacer", "nobody");
        assert_eq!(kartcraft::model::our_kart(), before);
    }

    #[test]
    fn the_settings_window_s_cvars_reach_the_arrow_the_camera_and_the_music() {
        let mut app = app();
        set(&mut app, "KartArrowX", "0.25");
        set(&mut app, "kartarrowy", "0.75");
        set(&mut app, "KartArrowSize", "9");
        set(&mut app, "KartArrowShow", "0");
        set(&mut app, "KartArrowMove", "1");
        let hud = *app.world().resource::<GuideHud>();
        assert_eq!((hud.x, hud.y), (0.25, 0.75));
        assert_eq!(
            hud.scale,
            GuideHud::MAX_SCALE,
            "an out-of-range size is clamped"
        );
        assert!(!hud.shown && hud.editing);

        set(&mut app, "KartChaseCamera", "0");
        set(&mut app, "KartMusicVolume", "0.4");
        let settings = *app.world().resource::<KartSettings>();
        assert!(!settings.chase_camera && !settings.chase_camera());
        assert_eq!(settings.music_level(), 0.4);
        set(&mut app, "KartMusic", "0");
        assert_eq!(app.world().resource::<KartSettings>().music_level(), 0.0);
        assert!(app.world().resource::<KartSettings>().ghost);
        set(&mut app, "KartGhost", "0");
        assert!(!app.world().resource::<KartSettings>().ghost);
    }
}
