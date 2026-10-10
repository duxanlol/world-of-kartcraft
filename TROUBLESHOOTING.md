# Troubleshooting

Start with the doctor: `powershell -ExecutionPolicy Bypass -File setup\setup.ps1 -Doctor` on
Windows, `setup/setup.sh --doctor` on Linux. Setup also writes everything it did to `setup.log`.

The commands below use `/`; on Windows write `server\compose.yaml` and so on.

## Setup

**"MISSING ..." in the doctor.** Run the `fix:` line printed under it, open a new terminal, run
setup again.

**"Data has no dbc.MPQ".** Point setup at the WoW folder itself (the one containing `Data/`), and
use an English 1.12.1 client. To pick a different folder later: delete the `WoW` link
(`rm WoW`, or `rmdir WoW` on Windows, which removes only the junction) and run setup again.

**"... does not exist" or the kart conversion fails.** Setup needs the `.bin` of a raw
(2352-byte sector) image of the US disc, SCUS-94426. European or Japanese discs, `.iso` files and
compressed images (`.chd`, `.pbp`) don't work. To point setup at a different image: delete
`.setup/ctr-path` (and `kart-data/` if it was half written) and run setup again.

**The server image build fails with "patch does not apply".** The `server/` checkout has CRLF line
endings (git's `autocrlf` on Windows), so the Karting patch doesn't match. Current setup converts
the checkout to LF by itself; with an older checkout do it by hand and run setup again:

```sh
git -C server config core.autocrlf false
git -C server rm --cached -r -q .
git -C server reset --hard
```

**The server image build fails with "unknown flag: exclude".** An older Docker that builds
without `COPY --exclude`. Current setup puts `# syntax=docker/dockerfile:1.7-labs` at the top of
the Dockerfile it builds, which fetches a frontend that understands it; update the repo and run
setup again, or update Docker.

**Map extraction seems stuck.** It takes hours, and the last two maps (the continents) take most
of it. If a CPU core or two is busy (Task Manager, or `top`), it's working.

**"the world server did not come up".** Look at
`docker compose -p world-of-kartcraft -f server/compose.yaml logs mangosd`. If the map extraction
was interrupted, delete `server/storage/mangosd/extracted-data/` and run setup again.

**The build failed.** Read the first `error:` line. On Linux, missing `alsa` or `libudev` means
the development packages from the doctor's fix line aren't installed.

## In game

**The login screen has no graphics, or the game says "no WoW install found".** The `WoW` link is
wrong: `WoW/Data` should list `.MPQ` files. Delete the link and run setup again.

**Can't log in.** Is the server up?
`docker compose -p world-of-kartcraft -f server/compose.yaml ps` should show `mangosd` and
`realmd` as healthy. To reset your password run setup again after deleting `.setup/account`
(it asks for the name and password again), or on Linux
`python3 tools/make_account.py <name> <newpassword>`.

**The kart is invisible.** `kart-data/kart.json` is missing: the log (the terminal you started
the game from) says `kart: no kart-data/kart.json`. Run setup again to convert it, or, if
`kart-data/` lives somewhere else, set `WOW_KART_DATA` to that folder. With `WOW_KART` set, the
game looks for `kart-<name>.json` there first.

**The controller does nothing.** Look in the log for `kart: controller connected: <name>`. PS5,
PS4 and Xbox controllers work natively; if the line isn't there, the game doesn't see the pad:
reconnect it (a cable is the surest), and close Steam or turn Steam Input off for the pad if Steam
is running. The keyboard always works as a fallback (W/S, A/D, Space, Shift; see the README).

**J does nothing.** You need to be in the world with a character. With the Karting profession J
casts Summon Kart like a mount, so anything that stops a mount cast stops it too.

## Windows

**"running scripts is disabled on this system".** Use `setup.bat`, which runs the script with
`-ExecutionPolicy Bypass`, instead of starting `setup.ps1` directly.

**"Docker Desktop running" is missing.** Start Docker Desktop and wait until it says the engine is
running, then run setup again. It needs virtualization enabled in your BIOS and WSL 2.

**Setup stops on a Docker warning.** Older copies of `setup.ps1` treated anything Docker printed
to stderr as an error. Update the repo: setup now decides only on the exit code.

**The build fails with linker errors (`link.exe` not found).** The Visual Studio C++ build tools
are missing; run the BuildTools `winget` line from INSTALL.md and open a new terminal.

**The build fails with "path too long".** Move the repo to a short path such as `C:\kartcraft`.

**The build is very slow.** Antivirus scanning `target\` slows Rust builds a lot; add the repo
folder as an exclusion in Windows Security if you're comfortable with that.

## Linux

**"Docker running and usable by you" is missing.** Start Docker
(`sudo systemctl enable --now docker`) and add yourself to the `docker` group
(`sudo usermod -aG docker $USER`), then **log out and back in**. On NixOS add your user to
`extraGroups = [ "docker" ]`.

**The pad isn't found.** Check `ls /dev/input/by-id/*event-joystick`; if it lists nothing, the
system doesn't see the pad either.
