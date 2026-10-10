# benilla's `gilrs-core` fork

Upstream: [`gilrs`](https://gitlab.com/gilrs-project/gilrs), MIT OR Apache-2.0, version `0.6.7`, copied
from crates.io as published and wired in through `[patch.crates-io]` in the workspace root.

The changes are all in `src/platform/windows_wgi/gamepad.rs`. Wine (and so Proton and the
Steam Deck) returns `E_NOTIMPL` for `NonRoamableId`, which upstream unwrapped (a panic the moment
a controller was seen) and used as the key for each pad's readings (so the pad was never read).
`controller_id()` falls back to the COM pointer, stable while a pad stays plugged in; the
`AxisCount`/`ButtonCount`/`SwitchCount` calls no longer `unwrap()` either.
