//! Kartcraft's settings as CVars, the channel the layer's settings window writes
//! (`assets/ui/KartcraftFrames.xml`, `/kartcraft`): `SetCVar` there, an observer here, and the
//! registry persists them in `config.toml` as it does every other row (`crate::cvars`).
//!
//! - `KartArrowShow`, `KartArrowSize`, `KartArrowX`, `KartArrowY`: the race's guide arrow
//!   ([`GuideHud`]); `KartArrowMove` is its placement mode, held for the session only.
//! - `KartChaseCamera`: CTR's chase camera, else our own follow camera.
//! - `KartFarCamera`: CTR's far chase camera (its L2), else the near one; the camera binding
//!   (`KARTCRAFT_CAMERA`) flips it while driving, so it persists.
//! - `KartMusic`, `KartMusicVolume`: CTR's race music, on top of the client's music volume.
//! - `KartClass`, `KartRacer`: the garage's engine class (as the Karting skill allows it,
//!   [`super::garage`]) and whose kart we drive; both apply at once, while driving too.
//! - `KartGhost`: the ghost of our best run on the course races us ([`super::ghost`]); the runs
//!   are recorded and kept either way.
//! - `KartTerrain`: the kart drives on CTR's grounds (grass and dirt hold it to 140/256 of its
//!   top speed, [`super::ground`]); off, as asphalt everywhere. Per player, client-side.
//! - `KartHideHud`: CTR's HUD and the guide arrow step aside for a clean picture, with the WoW
//!   interface (the layer's clean screen, `/kartclean`); held for the session only.
//! - `KartFreeCam`: the free camera for filming the kart (`/kartfree`, [`super::freecam`]):
//!   `off`, `fly`, `tripod`, `low` or `orbit`; held for the session only.
//! - `KartGateHeight`: how tall a race checkpoint's sheet of light stands (yards, 0 to 40; 0 leaves
//!   the beams at its ends alone). How it looks only: the gate passed is as wide as ever.
//! - `KartWowVoices`: the racers speak in their WoW characters' voices (wound vocals and
//!   text-emote voices, [`kartcraft::voice::WowLine`]) instead of CTR's cast, on by default.
//! - `KartRumble`: the controller shakes on crashes, hard landings and wall scrapes
//!   ([`kartcraft::rumble`]), as CTR's vibration does, on by default.
//! - `KartPadTest`: the controller test window (`/kartpad`) is open, so the pad's state is fed
//!   to it; `KartPadRumble`: `strong` or `weak` shakes that motor once, from the test's buttons.
//!   Both held for the session only.
//! - `KartCam*`: the chase camera's shape, the camera window's (`/kartcam`), one CVar a
//!   [`kartcraft::camera::CameraTuning`] field ([`CAMERA_KNOBS`]), read every tick while driving.
//!
//! `KartMinimapButton` and `KartMinimapAngle` are the layer's alone.

use bevy::prelude::*;
use kartcraft::camera::CameraTuning;

use super::race::GuideHud;

/// One of the camera window's sliders: its CVar and the range a value is clamped to (the
/// window's own slider spans it).
pub(crate) struct CameraKnob {
    pub(crate) cvar: &'static str,
    pub(crate) min: f32,
    pub(crate) max: f32,
}

const fn knob(cvar: &'static str, min: f32, max: f32) -> CameraKnob {
    CameraKnob { cvar, min, max }
}

/// The chase camera's numeric CVars, lengths in lu, angles in degrees, smoothing in seconds,
/// shares 0 to 1 ([`CameraTuning`] has each field's meaning and CTR's value); `KartCamTravel` and
/// `KartCamWallPull` are its switches.
pub(crate) const CAMERA_KNOBS: [CameraKnob; 19] = [
    knob("KartCamDistance", 64.0, 640.0),
    knob("KartCamSlowDistance", 64.0, 640.0),
    knob("KartCamHeight", 0.0, 256.0),
    knob("KartCamLookHeight", 0.0, 256.0),
    knob("KartCamPitch", -10.0, 45.0),
    knob("KartCamLookAhead", 0.0, 160.0),
    knob("KartCamClearance", 0.0, 192.0),
    knob("KartCamFarExtra", 0.0, 320.0),
    knob("KartCamFov", 30.0, 110.0),
    knob("KartCamAimDrop", 0.0, 160.0),
    knob("KartCamSwing", 0.0, 1.0),
    knob("KartCamFollow", 0.0, 1.0),
    knob("KartCamTurnSmoothing", 0.0, 1.0),
    knob("KartCamYawOffset", -180.0, 180.0),
    knob("KartCamSideOffset", -128.0, 128.0),
    knob("KartCamTilt", 0.0, 1.0),
    knob("KartCamTiltSmoothing", 0.0, 1.5),
    knob("KartCamBank", 0.0, 1.0),
    knob("KartCamTurbo", 0.0, 192.0),
];

/// The tuning field a [`CAMERA_KNOBS`] CVar sets.
pub(crate) fn camera_field<'a>(t: &'a mut CameraTuning, cvar: &str) -> Option<&'a mut f32> {
    Some(match cvar {
        "KartCamDistance" => &mut t.distance,
        "KartCamSlowDistance" => &mut t.slow_distance,
        "KartCamHeight" => &mut t.height,
        "KartCamLookHeight" => &mut t.look_height,
        "KartCamPitch" => &mut t.pitch,
        "KartCamLookAhead" => &mut t.look_ahead,
        "KartCamClearance" => &mut t.clearance,
        "KartCamFarExtra" => &mut t.far_extra,
        "KartCamFov" => &mut t.fov,
        "KartCamAimDrop" => &mut t.aim_drop,
        "KartCamSwing" => &mut t.swing,
        "KartCamFollow" => &mut t.follow,
        "KartCamTurnSmoothing" => &mut t.turn_smoothing,
        "KartCamYawOffset" => &mut t.yaw_offset,
        "KartCamSideOffset" => &mut t.side_offset,
        "KartCamTilt" => &mut t.tilt,
        "KartCamTiltSmoothing" => &mut t.tilt_smoothing,
        "KartCamBank" => &mut t.bank,
        "KartCamTurbo" => &mut t.turbo_push,
        _ => return None,
    })
}

/// The kart's own settings, as their CVars say.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub(crate) struct KartSettings {
    /// Drive with CTR's chase camera rather than our follow camera.
    pub(crate) chase_camera: bool,
    /// CTR's far chase camera rather than the near one.
    pub(crate) far_camera: bool,
    pub(crate) music: bool,
    /// The music's share of the client's music volume, 0 to 1.
    pub(crate) music_volume: f32,
    /// The engine class picked in the garage; the one driven is [`super::garage::effective`]'s.
    pub(crate) class: kartcraft::Class,
    /// Race the ghost of our best run on the course.
    pub(crate) ghost: bool,
    /// The kart drives on the ground as CTR's terrain table has it: off-road slows it
    /// ([`super::ground`]); off, every ground drives as asphalt.
    pub(crate) terrain: bool,
    /// Shake the controller ([`kartcraft::rumble`]).
    pub(crate) rumble: bool,
    /// The racers speak in their WoW characters' voices, not CTR's cast's.
    pub(crate) wow_voices: bool,
    /// A checkpoint gate's sheet of light, its height in yards.
    pub(crate) gate_height: f32,
    /// The controller test window is open.
    pub(crate) pad_test: bool,
    /// The chase camera's shape.
    pub(crate) camera: CameraTuning,
}

impl Default for KartSettings {
    fn default() -> Self {
        Self {
            chase_camera: true,
            far_camera: false,
            music: true,
            music_volume: 1.0,
            class: super::garage::TIERS[0].0,
            ghost: true,
            terrain: false,
            rumble: true,
            wow_voices: true,
            gate_height: GATE_HEIGHT_DEFAULT,
            pad_test: false,
            // CTR classic, everyone's to start with: the camera CTR's players know.
            camera: CameraTuning::CTR,
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

/// `KartGateHeight`'s default and its range (yards).
pub(crate) const GATE_HEIGHT_DEFAULT: f32 = 8.0;
pub(crate) const GATE_HEIGHT_MAX: f32 = 40.0;

/// The Kartcraft CVars' change callback.
pub(crate) fn on_cvar(
    ev: On<crate::cvars::CvarChanged>,
    mut hud: ResMut<GuideHud>,
    mut settings: ResMut<KartSettings>,
    mut free: ResMut<super::freecam::FreeCam>,
    mut hidden: ResMut<kartcraft::hud::HudHidden>,
    rumble: Option<ResMut<kartcraft::rumble::Rumble>>,
) {
    match ev.key().as_str() {
        "kartarrowshow" => hud.shown = ev.flag(),
        "kartarrowsize" => hud.scale = ev.num().clamp(GuideHud::MIN_SCALE, GuideHud::MAX_SCALE),
        "kartarrowx" => hud.x = ev.num().clamp(0.0, 1.0),
        "kartarrowy" => hud.y = ev.num().clamp(0.0, 1.0),
        "kartarrowmove" => hud.editing = ev.flag(),
        "kartarrowtilt" => hud.tilt = ev.num().clamp(0.0, GuideHud::MAX_TILT),
        "kartarrowfov" => hud.fov = ev.num().clamp(GuideHud::MIN_FOV, GuideHud::MAX_FOV),
        "kartchasecamera" => settings.chase_camera = ev.flag(),
        "kartfarcamera" => settings.far_camera = ev.flag(),
        "kartmusic" => settings.music = ev.flag(),
        "kartmusicvolume" => settings.music_volume = ev.num().clamp(0.0, 1.0),
        "kartghost" => settings.ghost = ev.flag(),
        "kartterrain" => settings.terrain = ev.flag(),
        "kartrumble" => settings.rumble = ev.flag(),
        "kartwowvoices" => settings.wow_voices = ev.flag(),
        "karthidehud" => {
            hidden.0 = ev.flag();
            hud.clean = ev.flag();
        }
        "kartfreecam" => free.set(super::freecam::FreeMode::named(&ev.new)),
        "kartgateheight" => settings.gate_height = ev.num().clamp(0.0, GATE_HEIGHT_MAX),
        "kartpadtest" => settings.pad_test = ev.flag(),
        "kartpadrumble" => {
            let (strong, weak) = match ev.new.as_str() {
                "strong" => (1.0, 0.0),
                "weak" => (0.0, 1.0),
                _ => return,
            };
            let Some(mut rumble) = rumble else { return };
            rumble.pulses.push(kartcraft::rumble::Pulse {
                strong,
                weak,
                seconds: 0.6,
            });
        }
        "kartcamwallpull" => settings.camera.wall_pull = ev.flag(),
        "kartcamtravel" => settings.camera.travel = ev.flag(),
        key if key.starts_with("kartcam") => {
            let Some(k) = CAMERA_KNOBS
                .iter()
                .find(|k| k.cvar.eq_ignore_ascii_case(key))
            else {
                return;
            };
            if let Some(field) = camera_field(&mut settings.camera, k.cvar) {
                *field = ev.num().clamp(k.min, k.max);
            }
        }
        "kartclass" => {
            settings.class = kartcraft::class_named(&ev.new).unwrap_or(super::garage::TIERS[0].0)
        }
        "kartpaint" | "karttrail" => {
            let n = ev.num().clamp(0.0, 255.0) as u8;
            let (paint, trail) = kartcraft::cosmetic::ours();
            if ev.key() == "kartpaint" {
                kartcraft::cosmetic::set_ours(n, trail);
            } else {
                kartcraft::cosmetic::set_ours(paint, n);
            }
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
            .init_resource::<super::super::freecam::FreeCam>()
            .init_resource::<kartcraft::hud::HudHidden>()
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
        assert_eq!(d("KartArrowTilt"), hud.tilt);
        assert_eq!(d("KartArrowFov"), hud.fov);
        let settings = KartSettings::default();
        assert_eq!(d("KartChaseCamera") != 0.0, settings.chase_camera);
        assert_eq!(d("KartFarCamera") != 0.0, settings.far_camera);
        assert_eq!(d("KartMusic") != 0.0, settings.music);
        assert_eq!(d("KartMusicVolume"), settings.music_volume);
        assert_eq!(d("KartGhost") != 0.0, settings.ghost);
        assert_eq!(d("KartTerrain") != 0.0, settings.terrain);
        assert!(
            !settings.terrain,
            "every ground drives as asphalt until asked"
        );
        assert_eq!(d("KartRumble") != 0.0, settings.rumble);
        assert_eq!(d("KartGateHeight"), settings.gate_height);
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
            super::super::garage::effective(picked, Some(1)),
            kartcraft::Class::Speed
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
        assert!(!settings.far_camera);
        set(&mut app, "KartFarCamera", "1");
        assert!(app.world().resource::<KartSettings>().far_camera);
        assert_eq!(settings.music_level(), 0.4);
        set(&mut app, "KartMusic", "0");
        assert_eq!(app.world().resource::<KartSettings>().music_level(), 0.0);
        assert!(app.world().resource::<KartSettings>().ghost);
        set(&mut app, "KartGhost", "0");
        assert!(!app.world().resource::<KartSettings>().ghost);
        assert!(
            !app.world().resource::<KartSettings>().terrain,
            "off by default"
        );
        set(&mut app, "KartTerrain", "1");
        assert!(app.world().resource::<KartSettings>().terrain);
        set(&mut app, "kartterrain", "0");
        assert!(!app.world().resource::<KartSettings>().terrain);
        assert!(app.world().resource::<KartSettings>().rumble);
        set(&mut app, "KartRumble", "0");
        assert!(!app.world().resource::<KartSettings>().rumble);
        use super::super::freecam::{FreeCam, FreeMode};
        set(&mut app, "KartFreeCam", "orbit");
        assert_eq!(
            app.world().resource::<FreeCam>().mode(),
            FreeMode::Locked(kartcraft::camera::CineMode::Orbit)
        );
        set(&mut app, "KartFreeCam", "off");
        assert_eq!(app.world().resource::<FreeCam>().mode(), FreeMode::Off);
        set(&mut app, "KartHideHud", "1");
        assert!(app.world().resource::<kartcraft::hud::HudHidden>().0);
        assert!(app.world().resource::<GuideHud>().clean);
        set(&mut app, "KartHideHud", "0");
        assert!(!app.world().resource::<kartcraft::hud::HudHidden>().0);
        assert!(!app.world().resource::<GuideHud>().clean);
        set(&mut app, "KartGateHeight", "3.5");
        assert_eq!(app.world().resource::<KartSettings>().gate_height, 3.5);
        set(&mut app, "KartGateHeight", "500");
        assert_eq!(
            app.world().resource::<KartSettings>().gate_height,
            GATE_HEIGHT_MAX,
            "an out-of-range height is clamped"
        );
    }

    fn registered(name: &str) -> &'static str {
        REGISTERED
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("{name} is registered"))
            .default
    }

    #[test]
    fn the_camera_cvars_default_to_ctr_classic_inside_their_ranges() {
        let mut from_cvars = CameraTuning::CTR;
        for k in &CAMERA_KNOBS {
            let value: f32 = registered(k.cvar).parse().unwrap();
            assert!(k.min <= value && value <= k.max, "{}", k.cvar);
            *camera_field(&mut from_cvars, k.cvar).unwrap() = value;
            // CTR's own value is in range too, so CTR classic is a setting.
            let mut ctr = CameraTuning::CTR;
            let ctr = *camera_field(&mut ctr, k.cvar).unwrap();
            assert!(k.min <= ctr && ctr <= k.max, "{} CTR {ctr}", k.cvar);
        }
        from_cvars.wall_pull = registered("KartCamWallPull") != "0";
        from_cvars.travel = registered("KartCamTravel") != "0";
        assert_eq!(from_cvars, CameraTuning::CTR);
        assert_eq!(KartSettings::default().camera, CameraTuning::CTR);
    }

    #[test]
    fn each_camera_cvar_reaches_the_tuning_clamped() {
        let mut app = app();
        for (i, k) in CAMERA_KNOBS.iter().enumerate() {
            // A value inside the range, distinct per knob, in the observer's lowercase too.
            let value = k.min + (k.max - k.min) * (i as f32 + 1.0) / 20.0;
            let name = if i % 2 == 0 {
                k.cvar.to_string()
            } else {
                k.cvar.to_ascii_lowercase()
            };
            set(&mut app, &name, &value.to_string());
            let mut tuning = app.world().resource::<KartSettings>().camera;
            assert_eq!(*camera_field(&mut tuning, k.cvar).unwrap(), value, "{name}");
            set(&mut app, k.cvar, "100000");
            let mut tuning = app.world().resource::<KartSettings>().camera;
            assert_eq!(*camera_field(&mut tuning, k.cvar).unwrap(), k.max, "{name}");
        }
        set(&mut app, "KartCamWallPull", "0");
        assert!(!app.world().resource::<KartSettings>().camera.wall_pull);
        set(&mut app, "kartcamwallpull", "1");
        assert!(app.world().resource::<KartSettings>().camera.wall_pull);
        set(&mut app, "KartCamTravel", "1");
        assert!(app.world().resource::<KartSettings>().camera.travel);
        set(&mut app, "kartcamtravel", "0");
        assert!(!app.world().resource::<KartSettings>().camera.travel);
        // Nothing else moved the far or chase camera.
        let settings = app.world().resource::<KartSettings>();
        assert!(settings.chase_camera && !settings.far_camera);
    }
}
