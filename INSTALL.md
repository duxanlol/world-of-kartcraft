# Installing World of Kartcraft

`setup.bat` (Windows) or `setup/setup.sh` (Linux) does all of this for you. This page explains
what it does and what it asks, so you know what to expect. If a step fails, setup stops, says what
to fix, and continues from that step when you run it again. Everything it does is logged to
`setup.log`.

Windows is the platform setup is tested on; the Linux script does the same steps.

## 1. Before you start

You need:

| | Where it comes from |
|---|---|
| WoW 1.12.1, English, build 5875 | Your own copy. The folder with `WoW.exe` and `Data/` |
| Crash Team Racing (PlayStation, USA, SCUS-94426) | Your own disc, as a raw `.bin`/`.cue` image (2352-byte sectors, as most ripping tools write it). An `.iso` or a compressed format (`.chd`, `.pbp`) does not work |
| A controller | A PS5 or PS4 controller works natively (no Xbox emulation), as does an Xbox controller; the keyboard works too |
| Docker with the compose plugin | Docker Desktop on Windows, your distribution's packages on Linux |
| ~20 GB free | WoW, the server's map data, the build |

Then run the doctor. It checks everything below and prints the exact install command if
something is missing:

- Windows: `powershell -ExecutionPolicy Bypass -File setup\setup.ps1 -Doctor`. It checks Rust,
  the Visual Studio C++ build tools, git, Docker Desktop (installed and running) and free disk.
  No Python is needed on Windows.
- Linux: `setup/setup.sh --doctor`. It checks Rust (via [rustup](https://rustup.rs)), a C
  compiler, pkg-config, ALSA and libudev development files, git, curl, Python 3 (for
  `tools/make_account.py`), and Docker installed, running, and usable without `sudo` (you are in
  the `docker` group; log out and back in after adding yourself).

On NixOS the scripts enter the repo's `shell.nix` by themselves; you only need Docker enabled in
your system configuration.

## 2. What setup does

1. **Checks your machine** (the doctor above).
2. **Links your WoW client.** It asks for your WoW folder and checks it has the 1.12.1 data
   files. The game reads it directly; nothing in it is changed.
3. **Converts the kart from your CTR disc.** It asks for the disc image (the `.bin`; pointing it
   at the `.cue` works too) and runs `ctr-extract`, which reads the kart models and textures off
   the disc and writes them to `kart-data/`. The first time, most of this step is compiling the
   extractor.
4. **Sets up the local server** ([vmangos](https://github.com/vmangos/core) through
   [vmangos-deploy](https://github.com/mserajnik/vmangos-deploy)) in `server/`, a clone of
   vmangos-deploy at a fixed version.
5. **Builds the server with the Karting profession.** vmangos is compiled into a local Docker
   image with `setup/vmangos/kartcraft.patch` applied (the image is tagged with the patch's hash,
   so a changed patch builds a new one). This takes a while the first time.
6. **Extracts map data from your WoW client** for the server. **The extraction takes several
   hours** and uses a couple of CPU cores; leave it running. It only happens once.
7. **Starts the server** and waits until the world server is healthy. Before that it copies the
   Race Marshals (`setup/vmangos/sql/kartcraft.sql`, the NPCs at each course start) into
   `server/storage/database/custom-sql/`, which the database runs on every start; a changed file
   restarts the database ([docs/courses.md](docs/courses.md#race-marshals)).
8. **Creates your game account.** It asks for a name and a password. The account is a GM.
9. **Builds the game**: `benilla` (the WoW client with the kart) and `kart-demo` (the offline
   demo). The first build takes several minutes.

Afterwards, start the game with `play.bat` (Windows) or `./play.sh` (Linux).

## 3. Windows

1. Install the tools. In a terminal (PowerShell or Command Prompt):

   ```bat
   winget install -e --id Git.Git
   winget install -e --id Rustlang.Rustup
   winget install -e --id Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
   winget install -e --id Docker.DockerDesktop
   ```

   Restart Windows afterwards, then start Docker Desktop and let it finish starting (it uses
   WSL 2; it will ask to set that up the first time).
2. Clone to a short path (long paths can break the build) and run setup:

   ```bat
   git clone <this repository> C:\kartcraft
   cd C:\kartcraft
   setup.bat
   ```

   You can also pass the paths up front:
   `powershell -ExecutionPolicy Bypass -File setup\setup.ps1 -Wow D:\WoW -Ctr D:\CTR.bin`.
3. Start the game with `play.bat`.

The Windows setup takes care of a few Windows-specific snags by itself: it keeps the `server\`
checkout in LF line endings whatever your git `autocrlf` setting is (the server's patches and
scripts run inside Linux containers), it builds the server image with
`# syntax=docker/dockerfile:1.7-labs` so older Docker Desktop versions understand the Dockerfile,
and it creates the account itself, without Python. `WoW` is a junction (a folder link) instead of
a symlink.

## 4. Linux

```sh
git clone <this repository> kartcraft
cd kartcraft
setup/setup.sh                      # or: setup/setup.sh --wow ~/WoW --ctr ~/CTR.bin
./play.sh
```

## 5. The offline demo

`demo.bat` (Windows) or `target/release/kartcraft-demo` (Linux) drops the kart into Northshire
straight from your WoW install: no server, login or character. It needs only `WoW` and
`kart-data/`, so it works as soon as step 3 of setup is done, which makes it a quick way to check
the kart and your controller before the map extraction finishes.

## 6. Where things are

| Folder | What | Safe to delete? |
|---|---|---|
| `WoW` | a link to your WoW folder | yes, setup asks again |
| `kart-data/` | the kart converted from your CTR disc | yes, setup converts again |
| `server/` | the server, its database (your characters) and map data | only if you want to start over: characters are lost and the maps re-extract for hours |
| `benilla-config/` | game settings, keybindings, addons (`benilla-config/AddOns/`) | yes, settings reset |
| `.setup/`, `setup.log` | setup's state (paths, account name, the server build context) and log | yes |

## 7. Everyday use

- Play: `play.bat` / `./play.sh` (starts the server if it isn't running).
- Stop the server: `docker compose -p world-of-kartcraft -f server/compose.yaml down`
  (`server\compose.yaml` on Windows).
- Update: `git pull`, then run setup again (it only redoes what changed; a changed Karting patch
  builds a new server image).
- New account or new password: run setup again after deleting `.setup/account`, or on Linux
  `python3 tools/make_account.py <name> <password>`.

## 8. Playing with friends

Friends don't need the repository, Rust or Docker, only their own WoW 1.12.1 client and CTR disc
image. You run the server and send them a ready-built client folder.

**The server.** You can run it on this PC or on a cloud machine:

- *This PC:* in `server\compose.yaml`, set `VMANGOS_REALMLIST_ADDRESS` to your public IP, then
  restart the server. Forward TCP 3724 (login) and 8085 (world) on your router.
- *A cloud VM* (any Linux host with Docker, e.g. Azure):
  1. Copy `server\` there: `compose.yaml`, `config\` and `storage\` (with
     `storage\database\custom-sql\kartcraft.sql`, the Race Marshals; after updating that file,
     restart the database container, then the world server).
  2. Copy the server image with `docker save` and `docker load`.
  3. Optionally copy the `world-of-kartcraft_vmangos-database` volume to keep your characters.
  4. Start it with `docker compose -p world-of-kartcraft up -d`.
  5. Open 3724 and 8085 in the host's firewall.

  The realm address column holds at most 32 characters, so use the VM's static IP, not a long
  cloud DNS name.

**Accounts.** Players can make their own on a sign-up page with a captcha and no email
(`setup/register/`, see its README), which runs beside the server. Or `account.bat` makes an account for a friend, an ordinary player (`-Gm` for an admin).
It works on the local server, or on a remote one over SSH: give it
`-Remote user@host -Key C:\path\to\key`, or save those two values as the two lines of
`.setup\remote`.

**The client.** `dist\publish.bat` builds the client and writes `dist\WorldOfKartcraft.zip`
(about 75 MB, send it once to each friend) and `dist\updates\`. `dist\publish.bat -NoBuild`
reuses the last build. `.setup\publish.cfg` (yours, never committed) presets the package, one
`KEY=value` per line:

- `SERVER=host`: the game server, so friends aren't asked for it.
- `UPDATE_URL=https://...`: where launchers fetch updates. The default is `http://SERVER:8080`,
  which `dist\serve-updates.bat` serves from this PC; forward 8080 if you use that.
- `AZURE_STORAGE=name`: publish also uploads the updates to that Azure storage account's static
  website (`az storage blob service-properties update --static-website`). `UPDATE_URL` is then
  that site's address.

**Updates.** After a change, run `dist\publish.bat` again. On every start, a friend's launcher
compares its files with the published `manifest.json` (SHA-256 per file) and downloads only the
files that changed, gzip-compressed and hash-checked. The launcher replaces itself too. When
`ctr-extract.exe` changes, it converts their disc again. If no update host is reachable, the game
starts as it is.
