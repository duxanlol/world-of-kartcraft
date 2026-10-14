//! World of Kartcraft's icon on the game window (its title bar, and on Windows its taskbar button
//! and Alt-Tab entry), from `dist/icon.png`, the WoK badge the Windows binaries also carry as
//! their resource icon (`benilla_buildstamp::windows_icon`). Set once, through winit, as soon as
//! the primary window exists. macOS shows a bundle's `.icns` in the Dock and takes no window icon,
//! so it is left out there.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

/// The badge, 256 px, PNG.
const ICON_PNG: &[u8] = include_bytes!("../../../dist/icon.png");

/// The icon as winit takes it; `None` should the PNG not decode.
fn icon() -> Option<winit::window::Icon> {
    let image = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    winit::window::Icon::from_rgba(image.into_raw(), width, height).ok()
}

/// Gives the primary window the icon, once; winit's windows live on the main thread.
fn apply(
    _main_thread: bevy::ecs::system::NonSendMarker,
    primary: Query<Entity, With<PrimaryWindow>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Ok(entity) = primary.single() else {
        return;
    };
    bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        if let Some(window) = windows.get_window(entity) {
            window.set_window_icon(icon());
            // Windows takes the taskbar's and Alt-Tab's (the big one) apart from the title bar's.
            #[cfg(windows)]
            {
                use winit::platform::windows::WindowExtWindows;
                window.set_taskbar_icon(icon());
            }
            *done = true;
        }
    });
}

pub(crate) fn plugin(app: &mut App) {
    if !cfg!(target_os = "macos") {
        app.add_systems(Update, apply);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_badge_decodes_into_a_window_icon() {
        assert!(super::icon().is_some());
    }
}
