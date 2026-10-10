# benilla's `gilrs-core` fork

Upstream: [`gilrs`](https://gitlab.com/gilrs-project/gilrs), MIT OR Apache-2.0, version `0.6.7`, copied
from crates.io as published and wired in through `[patch.crates-io]` in the workspace root.

The only change is in `src/platform/windows_wgi/gamepad.rs`: `NonRoamableId()` and the
`AxisCount`/`ButtonCount`/`SwitchCount` calls no longer `unwrap()`. Wine (and so Proton and the
Steam Deck) returns `E_NOTIMPL` for `NonRoamableId`, and upstream panicked the moment a controller
was seen. An empty id only means a reconnected pad is not matched to its old slot.
