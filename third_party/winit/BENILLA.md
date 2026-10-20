# benilla's `winit` fork

Upstream: [`rust-windowing/winit`](https://github.com/rust-windowing/winit), Apache-2.0, version
`0.30.13`, copied from crates.io as published (`src/`, `Cargo.toml`, `build.rs`, `README.md`,
`LICENSE`), its `[[example]]` and `[[test]]` targets dropped from `Cargo.toml` since their
folders are not copied. Wired in through `[patch.crates-io]` in the workspace root.

Everything changed is Android's, so no other platform builds anything different:

- `src/platform_impl/android/mouse.rs` (new) and its call in `mod.rs`'s `handle_input_event`.
  Upstream turns every motion event into a `Touch`, so a mouse gave one finger: no hover, no
  right button, no wheel. A mouse source now gives the desktop's events: `CursorMoved`,
  `MouseInput` per button from `ButtonPress`/`ButtonRelease`, `MouseWheel` from the scroll axes,
  `CursorEntered`/`CursorLeft`, and `DeviceEvent::MouseMotion` (a captured pointer's deltas, or
  the cursor's step).
- `src/platform/android.rs`, `gamepad` (new), and the two calls into it in `mod.rs`. Upstream
  turned a controller's sticks into touches and its buttons into keyboard keys. A source with the
  gamepad or joystick bit now queues its key codes and axes for the app to drain
  (`gamepad::take`); benilla turns them into Bevy gamepads (`benilla-app/src/android.rs`).

Every change is marked `benilla:` in a comment. To see them against upstream, diff this folder
against `~/.cargo/registry/src/*/winit-0.30.13`.
