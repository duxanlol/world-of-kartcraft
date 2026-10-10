# Contributing

Pull requests are welcome. This is a hobby project, so reviews may take a while.

## Setup scripts

`setup/setup.ps1` (run by `setup.bat`) is the tested path; `setup/setup.sh` does the same steps on
Linux. A change to either should say what you tested it on (Windows version or distribution, GPU,
controller) and must not break the other platform. The CI's Windows job checks that the game
compiles and the PowerShell script parses.

## Other changes

- Keep changes small and say how you checked them in game.
- No game files, ever: nothing from WoW or Crash Team Racing, and nothing converted from them
  (`kart-data/` stays out of the repository).
- The WoW client part is [benilla](https://github.com/samwhosung/benilla). Fixes that aren't
  about karting belong upstream there.
- The driving model (`crates/ctr-kart`) follows CTR's decompilation; a change to how it drives
  should point at the decompiled code it matches ([`docs/ctr-physics-spec.md`](docs/ctr-physics-spec.md)).
- Licences: benilla's code is MIT or Apache 2.0; `crates/ctr-kart`, `crates/ctr-assets`,
  `crates/kartcraft` and `crates/kart-demo` are GPL-3.0-or-later, and so is a build of the whole.
  New code is offered under the licence of the part it goes into; see [NOTICE](NOTICE).
