# World of Kartcraft

Check the gameplay out here: https://youtu.be/eQ5wpt6eZOc

Crash Team Racing karting in World of Warcraft 1.12.1. Summon your kart like a mount and your
character climbs in, with CTR's own driving model: hop into powerslides, fire boosts when the
meter goes red, chain triple turbos and catch hang time off Azeroth's hills, anywhere in the
world. Your own character drives, in your own gear. Karting is a profession: bank turbo chains to
level it, earn experience and unlock kart abilities such as Turbo Boost and Aku Aku.

> **Unofficial fan project.** Not affiliated with Blizzard Entertainment, Sony Interactive
> Entertainment, Activision or Naughty Dog. **No game files are included.** You need your own
> copies of both games.

> **Vibe coded, just for laughs.** This whole thing was put together with an AI coding assistant
> (Claude Code) for fun, on top of other people's serious work (see Credits). Expect rough edges.
> Don't take it seriously.

It is a fork of [World of Skatecraft](https://github.com/Kimmo3223/world-of-skatecraft), which
does the same with Skate 3, built on [benilla](https://github.com/samwhosung/benilla), a
from-scratch WoW 1.12.1 client in Rust. The kart drives on a Rust port of CTR's driving model,
read out of the [CTR decompilation](https://github.com/CTR-tools/ctr-native)
([`docs/ctr-physics-spec.md`](docs/ctr-physics-spec.md)), and looks like CTR's kart because setup
pulls it off your own disc ([`docs/ctr-assets.md`](docs/ctr-assets.md)).

## Just want to play?

You don't need to build anything. On Windows 10 or 11, with your own WoW 1.12.1 client and CTR
disc image (US, `.bin` or `.iso`):

1. **Make an account** at **https://kartcraft-5d37.austriaeast.cloudapp.azure.com**. No email
   needed.
2. **Download `WorldOfKartcraft.zip`** from the
   [latest release](https://github.com/duxanlol/world-of-kartcraft/releases/latest) and unzip it
   anywhere.
3. **Double-click `Play Kartcraft.bat`.** The first time, it asks for your WoW folder and your CTR
   image. Then log in, make a character and press **J**.

It updates itself every time you start it. The release notes cover the controls, races and
common problems.

The rest of this page is for building the game from source or running your own server.

## What you need

- **Windows** or **Linux**.
- **World of Warcraft 1.12.1**, English client (build 5875).
- **Crash Team Racing for the PlayStation**, the US release (SCUS-94426), as a raw `.bin`/`.cue`
  disc image.
- A **PS5 or PS4 controller** (it works natively, no Xbox emulation), an Xbox controller, or the
  keyboard.
- **Docker**, about **20 GB** free disk, and a few hours once for the server's map extraction.

## Install

Windows: install [Git](https://git-scm.com), then in a terminal:

```bat
git clone https://github.com/duxanlol/world-of-kartcraft C:\kartcraft
cd C:\kartcraft
setup.bat
```

Linux:

```sh
git clone https://github.com/duxanlol/world-of-kartcraft kartcraft
cd kartcraft
setup/setup.sh
```

Setup checks your machine and tells you exactly what to install if something is missing, asks for
your WoW folder and your CTR disc image, pulls the kart off the disc, builds the local server with
the Karting profession, creates your game account and builds the game. Step-by-step details:
[INSTALL.md](INSTALL.md).

## Play

```sh
play.bat         # Windows
./play.sh        # Linux
```

Log in with the account you made during setup and create a character, then press **J**.

**No server yet?** `demo.bat` (or `target/release/kartcraft-demo`) drops the kart into
Northshire straight from your WoW install, no server, login or character, to feel the driving.

| | PlayStation | Xbox | Keyboard |
|---|---|---|---|
| Gas | ✕ | A | W / ↑ |
| Brake; hold to reverse | □ | X | S / ↓ |
| Steer | left stick | left stick | A / D |
| Hop; hold through a turn to **powerslide** | R1 | RB | Space |
| **Boost** while sliding, when the meter goes red | L1 | LB | Shift |
| Abilities: Wumpa Juice, Turbo Boost, Super Engine, Aku Aku | D-pad ↑ → ↓ ← | D-pad | 1–4 |
| Climb in / out | | | **J** (or Summon Kart in your spellbook) |

The keyboard's keys are key bindings: change them in the options window's **Key Bindings**, under
**World of Kartcraft**, which also has Fire item (E), Race Finder, Leave race, Kartcraft settings
and Kart garage. While you drive the driving keys are the kart's alone (1–4 cast its abilities,
not your action bar); out of the kart W, A, S, D and 1–4 are WoW's as ever.

Everything else is WoW's own interface. The **Kartcraft button on the minimap** (the
mechanostrider) opens the **Race Finder** on a click; a right-click gives a menu with Race Finder,
Leave race, Leave queue (while queued), Kart garage, Kartcraft settings and Hide minimap button. Drag it to move
it round the minimap. **Kartcraft settings** (also `/kartcraft`) has the guide arrow (show it, its
size, Move arrow to drag it where you want it, Reset arrow), CTR's chase camera or your WoW camera,
the race music and its volume, and the minimap button itself. Slash commands: `/racefinder`,
`/leaverace` (`/lr`), `/kartcraft`, `/garage`.

**Kart garage** (minimap menu, or `/garage`) picks your kart. The **engine class** is CTR's, and
your Karting skill opens them: **Turning** from the start (the slowest, and the easiest to steer),
**Balanced** at 75, **Acceleration** at 150 and **Speed** at 225. A locked class is greyed with
the skill it needs; if your pick is ever above your skill you drive the best one you have. The
**kart** is any CTR racer's whose kart setup converted from your disc, free to pick; the others
see it too. Both apply at once, even while you drive.

### How it drives

It is CTR's model, constants and all, in the engine class you pick in the garage:

- **Powerslide:** hop and hold the shoulder while you steer into the turn. The turbo meter under
  the kart fills; press the other shoulder in the **red** part for a boost. Up to three per slide,
  each faster than the last. Too early and the engine puffs; wait too long and the chain is over.
  Hold the steer in too long without boosting and you spin out.
- **Hang time:** a hop that keeps you in the air long enough pays a turbo when you land.
- **Walls:** hills too steep to drive are walls. A scrape slows you; a head-on hit at speed
  crashes you. Fall off the world and you are put back where you last drove, as Aku Aku would.

It looks and sounds like CTR too, all of it from your disc: the kart in your racer's colours,
the turbo flames (bigger with each boost of a slide), exhaust that **smokes dark in the boost
window** (CTR's "press now" cue), skid marks, CTR's chase camera, its HUD (speedometer, turbo
meter, hang-time meter, Wumpa count, lap and place), the engine and every kart sound, and the
16 race tracks' music, a different one each time you climb in (`WOW_KART_MUSIC=crash_cove` for
one, `off` for none).

### Karting, the profession

Every character knows **Summon Kart** (spellbook, General tab): it casts like a mount and seats
you in the kart until you cancel the buff or press **J** again. Karting is a secondary skill, 1 to
300, in your Skills tab. Boosts in a row make a **turbo chain**: each one is worth more than the
last, and the chain banks once you stop boosting for two seconds (a crash loses it). Each banked
chain raises the skill (bigger chains raise it faster) and gives your character experience; the
chain, its points, the experience and the skill show at the top of the screen. As the skill grows
you learn kart abilities, buffs to drag onto your action bar; they work only in the kart:

| Skill | Ability | Effect | Lasts | Cooldown |
|---|---|---|---|---|
| 1 | Summon Kart | drive | until cancelled | none |
| 75 | Wumpa Juice | juiced: every turbo burns hotter | 30 s | 2 min |
| 150 | Turbo Boost | CTR's turbo item | instant | 30 s |
| 225 | Super Engine | turbo whenever you hold the gas | 30 s | 3 min |
| 300 | Aku Aku | the mask's speed, and walls cannot stop you | 20 s | 5 min |

On a server without the profession **J** just climbs in and out. Your account is a GM, so chat
commands like `.tele stormwind`, `.levelup 59` and `.additem <id>` work.

### Driving together

Other drivers on your server are drawn in their own karts, in their racer's colours, with their
character in the seat, their powerslides, flames, smoke and engine. Karts bump each other as in
CTR. Each driver's kart state reaches the players within 150 yards through the server.

**Races.** Drive to a course's start and talk to the **Race Marshal** there: he hosts a race,
lets drivers near you join it and lets the host start it. CTR's start lights count down while you
rev on the grid as in CTR: the gas revs the engine, the meter shows the revs, and revs in the red
at GO give the start boost (pump the gas first for the big one; hold it too long and the engine
over-revs and gets none). Beams of light mark the
checkpoints, the next one bright; the server checks each one against where you really are. Lap
and place show on CTR's HUD, the music speeds up on the final lap, and a finish pays experience
and Karting skill by place. **Leave race** on the minimap button's menu (or `/leaverace`) leaves.

**Race Finder.** Click the Kartcraft minimap button anywhere (or type `/racefinder`, or ask a
Race Marshal), tick the courses you want (or Any course) and **Find race**,
like the dungeon finder. Once two or more players share a course (it waits up to 30 seconds for
more, up to eight), everyone gets a ready check, WoW's own popup: **Accept** or **Decline**. When all accept,
the server puts each of you in your kart and teleports you to a slot on the starting grid, and
the countdown starts. The courses, laid along Elwynn's roads (the guards'
own patrol routes):

| Course | Start | Laps |
|---|---|---|
| Northshire Sprint | in front of Northshire Abbey | sprint to Goldshire |
| Elwynn Loop | Goldshire, by the inn | 3 |
| Road to Stormwind | Goldshire, by the inn | sprint to the city gate |

**Friends over the internet.** On your PC (the server): forward TCP ports 3724 and 8085 to it,
set `VMANGOS_REALMLIST_ADDRESS` in `server/compose.yaml` to your public address and restart the
server (`play.bat` does), and make each friend an account (`setup.bat -NewAccount`, or
`tools/make_account.py` on Linux). Each friend needs their own WoW 1.12.1 and CTR disc, and only the game:
`setup.bat -ClientOnly` (no Docker), then `set WOW_HOST=<your address>` and `play.bat`.

### Tuning

| Variable | |
|---|---|
| `WOW_KART_CLASS` | the garage's class for a run: `turn`, `balanced`, `accel`, `speed` (still only as far as your skill goes) |
| `WOW_KART` | the garage's kart for a run: `crash`, `cortex`, `coco`, `tiny`, `polar`, `pura`, `pinstripe`, `roo`, `joe`, `pen`, `fake` |
| `WOW_KART_SCALE` | kart size against CTR's own, model and physics together (default 1) |
| `WOW_KART_SIZE` | the model alone against its physics (default 1.25 in the game, for a WoW driver) |
| `WOW_KART_MUSIC` | a race track's name (`crash_cove`, `roo_tubes`, ...) or `off` |
| `WOW_KART_SEAT` | `x,y,z` meters to move the driver off CTR's seat |
| `WOW_KART_CAM=wow` | start on your WoW camera instead of the chase camera |

## Help

This is a hobby project with **best-effort help only**. If something breaks, run the doctor
(`setup/setup.sh --doctor`, or on Windows `powershell -ExecutionPolicy Bypass -File
setup\setup.ps1 -Doctor`) and read [TROUBLESHOOTING.md](TROUBLESHOOTING.md).

## Credits

- **[World of Skatecraft](https://github.com/Kimmo3223/world-of-skatecraft)** by Kimmo3223: the
  project this one is a fork of, and its whole idea.
- **[benilla](https://github.com/samwhosung/benilla)** by samwhosung and contributors: the WoW
  1.12.1 client this is built on. Its own README is in [docs/BENILLA.md](docs/BENILLA.md).
- **[CTR-tools](https://github.com/CTR-tools)**: the CTR decompilation
  ([ctr-native](https://github.com/CTR-tools/ctr-native),
  [CTR-ModSDK](https://github.com/CTR-tools/CTR-ModSDK)) the driving model is ported from, and
  DCxDemo's [CTR-tools](https://github.com/CTR-tools/CTR-tools) format work the kart extractor
  follows.
- **[vmangos](https://github.com/vmangos/core)** and
  **[vmangos-deploy](https://github.com/mserajnik/vmangos-deploy)** (mserajnik): the local server.

## Legal

benilla's code is MIT or Apache 2.0 at your option. The parts ported from the CTR decompilation
(`crates/ctr-kart`, `crates/ctr-assets`, `crates/kartcraft`, `crates/kart-demo`) are GPL-3.0, as
the decompilation is, and so a build of the whole is GPL-3.0; see [NOTICE](NOTICE). No World of
Warcraft or Crash Team Racing content is included or distributed: the game reads your WoW install,
and setup converts your own CTR disc on your machine. World of Warcraft is a trademark of Blizzard
Entertainment, Inc.; Crash Team Racing and Crash Bandicoot are trademarks of Sony Interactive
Entertainment / Activision.
