# benilla's `android-activity` fork

Upstream: [`rust-mobile/android-activity`](https://github.com/rust-mobile/android-activity),
MIT OR Apache-2.0, version `0.6.1`, copied from crates.io as published and wired in through
`[patch.crates-io]` in the workspace root.

One change, in `src/native_activity/mod.rs`, marked `benilla:`. `show_soft_input` asked
`InputMethodManager.showSoftInput` for the window's decor view, from the app's own thread.
The decor view never holds the input focus, so Android ignored the request and the keyboard
never opened. It now calls the NDK's `ANativeActivity_showSoftInput`, which asks from
NativeActivity's content view on the UI thread. benilla opens the phone's keyboard for a text
box this way (`benilla-app/src/android.rs`, through winit's `set_ime_allowed`).
