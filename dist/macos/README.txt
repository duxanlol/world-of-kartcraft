WORLD OF KARTCRAFT FOR MAC
==========================

Crash Team Racing karting in World of Warcraft 1.12.1. Climb into a CTR kart (J), powerslide,
fire boosts when the meter goes red, chain turbos for experience and the Karting skill, and race
your friends through Elwynn Forest.

Unofficial fan project, not affiliated with Blizzard, Sony, Activision or Naughty Dog.
No game files are included: you need your own copies of both games.


WHAT YOU NEED
-------------
- A Mac with macOS 11 or later: Apple Silicon (M1 and up) or Intel, the same app runs on both.
- World of Warcraft 1.12.1, the English client (build 5875). Only its Data folder is read. A
  Windows install's folder copied over works; so does a Mac one that carries the same MPQs.
- Crash Team Racing for the PlayStation, the US release (SCUS-94426), as a raw .bin/.cue image.
- The address of a World of Kartcraft server (whoever sent you this runs one) and an account
  on it (they make it for you).
- Optional: a PS5, PS4 or Xbox controller over Bluetooth or USB, else the keyboard.


FIRST OPEN
----------
1. Unzip "World of Kartcraft.zip" and double-click "World of Kartcraft". The first time, macOS
   says the app can't be opened because it is from an unidentified developer: it is not signed
   with an Apple Developer ID.
   - macOS 14 and earlier: right-click (or Control-click) the app, choose Open, then Open again.
   - macOS 15 and later: close the message, open System Settings > Privacy & Security, scroll
     down to "World of Kartcraft was blocked", click Open Anyway, then Open.
   This is asked once.
2. The launcher moves itself into your Applications folder (~/Applications/World of Kartcraft)
   and opens from there; delete the one in Downloads when you like.
3. It looks for your WoW 1.12.1 folder and your CTR disc image (in Applications, Games,
   Downloads, Desktop and Documents) and asks for whatever it cannot find, with Browse….
4. It converts the kart, sounds, effects, HUD and music from your disc (a few seconds), checks
   the server for updates, and PLAY lights up.
Your answers are kept in ~/Library/Application Support/World of Kartcraft/kartcraft.cfg; delete
that file to be asked again. The game's own settings live in benilla-config in the same folder.
Nothing is ever written into your WoW folder.

Then log in with your account, pick the realm, create a character and press J.

The offline demo (no server, no account; just the kart over Northshire) is the kartcraft-demo
program inside the app: in Terminal,
  WOW_DATA="/path/to/your/WoW/Data" "~/Applications/World of Kartcraft.app/Contents/MacOS/kartcraft-demo"


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
  Quit                                          Cmd-Q

The keyboard keys are rebindable: options window, Key Bindings, World of Kartcraft, where a
Controller column rebinds the pad too. Kartcraft settings > Controller picks one pad when
several are connected. Everything else is as in the Windows README: the Race Finder on the
minimap button, /kartcraft, /kartcam, /garage, /racefinder, /tracks, /leaverace, /unstuck.


UPDATES
-------
Every start checks the server for a newer version and downloads only the files that changed,
into the app itself. If the server can't be reached, the game starts as it is. To get updates
from a different address, add a line UPDATE_URL=http://... to kartcraft.cfg (the Mac files are
in its macos/ folder).


LICENCE AND SOURCE
------------------
World of Kartcraft is built on benilla (MIT/Apache-2.0) and on a port of Crash Team Racing's
driving model from the CTR decompilation (GPL-3.0), so this build is distributed under the GNU
General Public License, version 3 or later. Its complete source code is in source.zip inside the
app (Contents/Resources); see NOTICE.txt and the licence files beside it.
