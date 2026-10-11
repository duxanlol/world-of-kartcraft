WORLD OF KARTCRAFT
==================

Crash Team Racing karting in World of Warcraft 1.12.1. Climb into a CTR kart (J), powerslide,
fire boosts when the meter goes red, chain turbos for experience and the Karting skill, and race
your friends through Elwynn Forest.

Unofficial fan project, not affiliated with Blizzard, Sony, Activision or Naughty Dog.
No game files are included: you need your own copies of both games.


WHAT YOU NEED
-------------
- Windows 10 or 11 (64-bit).
- World of Warcraft 1.12.1, the English client (build 5875). Only its Data folder is read.
- Crash Team Racing for the PlayStation, the US release (SCUS-94426), as a raw .bin/.cue image.
- The address of a World of Kartcraft server (whoever sent you this runs one) and an account
  on it (they make it for you).
- Optional: a PS5, PS4 or Xbox controller (it works natively), else the keyboard.


FIRST RUN
---------
Easiest: put your WoW 1.12.1 folder and your CTR .bin/.cue inside the World of Kartcraft folder
(or unzip World of Kartcraft inside your WoW folder), then double-click "Play Kartcraft.bat". It
finds both by itself. Otherwise it asks for:
  1. your WoW 1.12.1 folder,
  2. your CTR .bin file (it converts the kart, sounds, effects, HUD and music into kart-data\,
     which takes a few seconds),
  3. the server's address (unless your copy already knows it).
The answers are kept in kartcraft.cfg; delete that file to be asked again.

Then log in with your account, pick the realm, create a character and press J.

"Demo.bat" runs an offline demo (no server, no account): just the kart over Northshire.


CONTROLS
--------
                      PlayStation   Xbox        Keyboard
  Gas                 Cross         A           W / Up
  Brake (hold: back)  Square        X           S / Down
  Steer               left stick    left stick  A / D
  Hop / powerslide    R1            RB          Space (hold through a turn)
  Boost (meter red)   L1            LB          Shift
  Fire item           Circle        B           E
  Abilities           D-pad         D-pad       1-4
  Near / far camera   Select        View        C
  Look back (hold)    R3            RS click    X
  Climb in / out                                J (or Summon Kart, spellbook)

The keyboard keys are rebindable: options window, Key Bindings, World of Kartcraft (also Race
Finder, Leave race, Kartcraft settings and Kart garage). While you drive the driving keys are the
kart's alone (1-4 cast its abilities, not your action bar); out of the kart W, A, S, D, 1-4, C and
X are WoW's as ever.

Everything else is WoW's own interface. The Kartcraft button on the minimap (the mechanostrider)
opens the Race Finder on a click; right-click it for Race Finder, Leave race, Leave queue,
Kart garage, Kartcraft settings and Hide minimap button; drag it to move it round the minimap.
Kartcraft settings (or /kartcraft): the guide arrow (show, size, Move arrow, Reset arrow), CTR's
chase camera or your WoW camera, near or far, the race music and its volume, the ghost of your
best run, and the minimap button.
Slash commands: /racefinder, /tracks, /leaverace (/lr), /unstuck, /kartcraft, /garage.

In a race the guide arrow points at the next checkpoint, and a big arrow floats in the next
checkpoints pointing on to the one after it. A map of the track with every racer on it sits in the corner; drag it where you like.

Kart garage (or /garage): your engine class and your kart. Karting skill opens the classes:
Turning from the start (slowest, easiest to steer), Balanced at 75, Acceleration at 150, Speed
at 225; locked ones are greyed with the skill they need. Any CTR racer's kart is free to pick,
and other drivers see it. Both change at once, even while you drive.

Powerslide: hop and hold while steering into a turn; the turbo meter by the speedometer fills.
Press the other shoulder when it turns red for a boost, up to three per slide. The exhaust smokes
dark when it is time. Too early and the engine puffs; hold the turn too long and you spin out.


RACES
-----
Drive to a course's start and talk to the Race Marshal: he hosts a race, lets drivers near you
join it and lets the host start it. Rev the engine just before GO for a start boost. Beams of
light mark the checkpoints. Leave race (minimap menu) or /leaverace leaves.

Or click the minimap button anywhere for the Race Finder: tick courses (or Any course) and Find race. When two or
more players match (it waits up to 30 seconds for more), a ready check pops up: Accept or
Decline. When everyone accepts, you are put in your kart and teleported to the starting grid.
  Northshire Sprint   in front of Northshire Abbey, down to Goldshire
  Elwynn Loop         Goldshire, by the inn, 3 laps
  Road to Stormwind   Goldshire, by the inn, to the city gate


LICENCE AND SOURCE
------------------
World of Kartcraft is built on benilla (MIT/Apache-2.0) and on a port of Crash Team Racing's
driving model from the CTR decompilation (GPL-3.0), so this build is distributed under the GNU
General Public License, version 3 or later. Its complete source code is in source.zip, beside
this file; see NOTICE.txt and the licence files in it.


UPDATES
-------
"Play Kartcraft.bat" checks the server for a newer version every time it starts and downloads only
the files that changed. If the server can't be reached, the game starts as it is. To get updates
from a different address, add a line UPDATE_URL=http://... to kartcraft.cfg.


LINUX AND STEAM DECK
--------------------
Run "bash play-proton.sh" instead of the .bat (it needs curl and jq). It uses your Steam's Proton or
GE-Proton, or the one PROTON=/path/to/proton names, and asks the same questions on the first run.
