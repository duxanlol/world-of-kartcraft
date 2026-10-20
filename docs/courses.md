# Courses

Every race course is defined once, in `courses/courses.json`. The server's table and the
client's list are both made from it; nothing else holds course data.

## The file

```json
{
  "courses": [
    {
      "key": "elwynn",
      "name": "Elwynn Loop",
      "zone": "Elwynn Forest",
      "map": 0,
      "laps": 3,
      "note": "From Goldshire's inn past the farms and back.",
      "marshal": {"o": 4.712, "x": -9436, "y": 69, "z": 56.1},
      "checkpoints": [
        { "x": -9436, "y": 59 },
        { "x": -9640, "y": 150, "z": 58.2, "gate": 25 }
      ],
      "crates": [
        { "kind": "Q", "x": -9540, "y": 104 },
        { "kind": "W", "x": -9600, "y": 130, "z": 57.0 }
      ]
    }
  ]
}
```

- `key`: 1-32 of `a-z 0-9 _ -`, unique, not `any`. It is what the wire carries (`R\tH`, the
  Race Finder's queue list).
- `name`: shown to players. ASCII, no tabs, quotes or backslashes.
- `zone`, `note`: optional, for people; the note becomes the comment above the C++ entry.
- `series`: optional, the hand-made set a course belongs to (`"Grand Prix"`). The Race Finder
  offers it under its series instead of its zone, and the track browser names both.
- `map`: the map id (0 Eastern Kingdoms, 1 Kalimdor). Any other map is a dungeon's: the course
  is a dungeon course (below). `pack.py` gives a dungeon course its start height (the start's
  `z`, else its marshal's), the floor a teleport onto its grid lands on from outside, not a roof
  over it.
- `laps`: 0 for a sprint from the first checkpoint to the last; else laps back through the first.
- `scale`: optional, 0.2 to 3, default 1: the karts' and racers' size on the course. The world
  stays as it is, so a small kart makes it drive bigger (Stormwind's streets as a kart's city):
  the client sizes the kart and its physics by it for the race (`kartcraft::set_race_scale`; the
  session starts again at the new size, as at the race's grid), and the server sizes each
  racer's body as everyone sees it (`Kartcraft.cpp` "Race scale"), back to its own once out of
  the race. A kerb or a step keeps its height, so a small kart climbs less of one: line a small
  course round them. The track editor's Size buttons set a player track's, which it saves,
  publishes (`R\tPub`'s last field, the `PT` lines' last) and runs its test laps at (`R\tSz`
  asks the server to size the body for one).
- `checkpoints`: at least two, WoW world `x`, `y` (yards); the first is the start, where a
  driver hosts or joins. Optional `z` (the server finds the ground itself). Optional `gate`:
  the gate's half-width in yards. The client passes a checkpoint through a gate this wide
  (default 20); the server accepts a report within `max(45, gate)` yards.
- A checkpoint's optional `o`: the way the course runs through its gate (radians, a creature's
  facing), which the track editor sets when a gate is turned; without it the gate stands square
  between the legs in and out. Client-side only: the server takes a checkpoint by distance.
- `marshal`: where the course's Race Marshal stands (`x`, `y`, `z`, orientation `o`), written by
  `course-gen marshals` (below); courses that share a start share the spot. Required by `pack.py`.
- `crates`: optional. When present it replaces the layout the server builds along the
  segments: `kind` `F` (fruit crate), `Q` (weapon crate) or `W` (wumpa fruit), `x`, `y`, and an
  optional `z` (else the ground's).
- `pads`: optional turbo pads, `x`, `y`, an optional `z` (else the ground under the camera's
  height) and `o`, the way the pad points (radians, as a creature's facing). Client-side only:
  the client lays them on the ground and fires CTR's turbo pad while a kart is on one, only in a
  race or time trial on that course, never in the open world (a track's pads are its own, as
  its placed objects are, `kart/pads.rs`); the server and `pack.py` ignore them. `course-gen pads` writes them on each course's straights (legs of at
  least 90 yards turning under 20° at both ends, the straightest three, never on neighbouring
  legs, 30% along the leg).
- `doors`: optional, gameobject entries (the server's `gameobject_template`) a race on the course
  opens for as long as it runs: each is the one of that entry nearest the host when the race
  starts, if it is closed, and it closes again once the race is over (at the latest after the
  race's 20 minutes). For a door across the course that the dungeon keeps locked: the Stratholme
  Street Circuit's `[175352, 175353]`, the King's Square gates.
- Up to 1000 courses. The Race Finder offers them by zone (or series), in the order the file
  first names each; a queue line names one by that place (`z3`), so keep first appearances in
  order when adding courses (the exporter appends). The Grand Prix leads the file, so it heads
  the finder.

## Regenerating

After editing the file:

    python3 setup/vmangos/pack.py

This validates the file, writes `setup/vmangos/src/KartcraftCourses.h` (included by
`Kartcraft.cpp`) and `setup/vmangos/sql/kartcraft.sql` (the Race Marshals, below), and
refreshes the new-file hunks of `setup/vmangos/kartcraft.patch`, which the server image is built
from; rebuild the server image to take the change. `pack.py --check` exits non-zero when the
header, the SQL or the patch is stale. The client needs nothing: it embeds the file at compile
time (`crates/benilla-app/src/kart/race.rs`, `COURSES`), so a rebuild picks it up.
The checkpoints a race uses still reach the client from the server at the start (`R\tC`).

## Generated courses

`course-gen` (crates/course-gen) lays courses over a zone's terrain from the player's install
and writes only their checkpoints here: `course-gen export --candidates <run>/candidates.json
--all` adds a run's finalists (key `<zone>-<place>-<place>`, replacing a course of the same key),
and `course-gen batch --zones alliance,horde --out <dir>` runs zones and adds each one's
finalists as it finishes, never replacing a key already in the file.

### In a dungeon

`--zone` may name a dungeon map (`Map.dbc`'s name, "Stratholme"), which has no world map: its
whole map is the zone. A dungeon's streets are WMO faces over filler terrain (Stratholme is one
city WMO, `stratholme_b.wmo`, above flat ground sixty yards down), so its ground is the walkable
floor of its WMOs (`course_gen::wmo_floor`): every face the client's walking collision keeps, the
floors joined where a kart can drive from one to the next, the walls and the WMOs' own doodads
(carts, crates, lamp posts) solid, at a yard a pixel. Its area triggers are kept clear: one is an
exit (Stratholme's back gate) or starts a script. The streets are too narrow for the open-ground
generator, so a dungeon's circuit is named by its streets: `--via "x,y;x,y;..."`, waypoints in
driving order from the start, and course-gen routes, smooths and test-drives the line through
them. The Stratholme Street Circuit:

    course-gen courses --zone Stratholme --out <dir> --via "3539,-3389;3553,-3334;3613,-3336;3668,-3331;3713,-3429;3660,-3477;3633,-3501;3572,-3452;3556,-3435"

then `export`, rename the key and name, add its `doors`, and `marshals` (a start on a WMO floor
gets its marshal at the start's height, not the terrain's).

## From the in-game track editor

The track editor (the minimap's map button, `kart/track_edit.rs`) flies the free camera and hides
the game's interface for its own (`kart/track_edit_ui.rs`): Exit at the top left, Test lap and
Save at the top right, and named bars of icon buttons, each naming itself on hover: Editor
controls (Move, Undo, Redo), Checkpoints (place a checkpoint, Edit checkpoint, place the grid)
and Race type (sprint, one lap or three), and Objects: placeable models from the player's own
install (`kart/objects.rs`, the planks for now), by group, each a tile with its icon. An object
goes where the cursor points, turned with Q/E and the middle mouse (across yaws, up and down
pitches, with Shift rolls); Move drags one and Delete removes it, and Undo takes back gates and
objects in the order they were placed. Objects are solid: the kart drives on them in the editor,
in any test lap and in a race on a player track. CTR's own props join them, from the player's
own disc as the launcher converts it (`ctr-extract --props`, in `--all`: `kart-data/props.json`,
`ctr_assets::props`): the CTR Hazards (the armadillo, TNT and Nitro crates, barrels...), CTR
Pickups (the ? crate, Wumpa fruit, the relic...) and CTR Scenery groups, keyed `ctr-<name>`, at
the kart's own size. The Build group's pieces (`kart/pieces.rs`) fill the world's gaps: a ramp
(a wedge rising along its length) and a block, each in six materials of the install (wood,
cobble, dirt, paving, grate, stone; keyed `ramp-wood`, `block-stone`...), of any size from
0.5 to 200 yards each way, its texture repeating by the yard, never stretched. `[` `]`, `;` `'`
and `-` `=` size the one held or about to be placed (length, width, height); a saved track keeps
its `size` (`[length, width, height]`), and a publish sends it after the turn
(`key,x,y,z,yaw,pitch,roll,l,w,h`), which the server bounds and keeps. Its mesh and collider are
built at its size. The Boosts group's turbo pad (`turbo-pad`) is
no model and no collider: CTR's pad, draped on the ground under where it is placed and pointing
as it is yawed, fired as a course's pads are, and picked for Move and Delete by the aim over it. A saved track keeps them as `objects` (`model`,
`x`, `y`, `z`, `yaw`, `pitch`, `roll`, radians); a publish sends them as its last field
(`key,x,y,z,yaw,pitch,roll;...`, at most 40), which the server keeps with the track
(`kartcraft_user_tracks.objects`) and puts last on its `PT` lines, the list's and a race's.
`pack.py` and the official courses have none. What cannot be done
says why in a popup. It
opens on the Move cursor, which places nothing: a press on the start, a checkpoint or the grid
drags it (Q/E and the wheel turn and size a dragged gate). Edit checkpoint picks one with a
click and gives it a new number (1, the start, to the count; the others move up or down a
place), and Redo puts back what Undo took until a new gate is placed. The checkpoint tools place where the cursor points (a left click, or Fire at the screen's centre): the checkpoints in driving order, the
first placed the start, each gate turned with the middle mouse
held, and optionally a starting grid, facing the way the camera looks. A card on the left lists
the keys. Save asks for the track's name in a popup, which warns when it replaces one of ours.
A test lap asks the server to take the kart onto its grid (`R\tTG\t<x>\t<y>\t<o>\t<z>`, the
placed grid or a race's first slot behind the start, `z` the track's floor; within 1000 yards,
once every 10 seconds, the checks of a trip to a Race Marshal), runs the lights there and times
the lap on the client's own clock, with no records. A save, or Close, starts a new
draft. A saved track is one course object in this file's form, in
`benilla-config/kart-tracks/<key>.json`: `key`, `name`, `zone`, `map`, `laps`, `note`, the
`checkpoints` (each with the `z` it was placed at) and `grid` (`x`, `y`, `z`, `o`), with no
`marshal`. The editor works in a dungeon as in the open world: a track on a dungeon's map is
made, test-lapped (the grid trip stays in our copy of it) and published there. Its floor, the
grid's height or else the start's, goes with its publish (`R\tPub`'s last field, the `PT`
lines' last) and its test lap, so the server lands a trip onto its grid on the course's floor
among a dungeon's floors and roofs. The track browser lists the
saved tracks after the courses (right-click the editor's button, "My tracks", for them alone)
and runs a test lap of one, or Edit opens it in the editor again, the kart taken to its start
first by the test lap's trip (the camera flies once it lands; refused, it edits from where it is) (saved under the same name, it
replaces the file, as the save popup warns; a publish over one of ours of that name asks too). A publish or a take-down sends the author the fresh list at once. `grid` is the editor's alone for now: `pack.py` and the server
ignore it and line a race up behind the start. In the browser our own track's Delete takes its
published copy down (`R\tPubDN\t<name>`: ours of that name, as a publish replaces it) and removes
its file, and Publish sends it to the server (`R\tPub`, `Kartcraft.cpp` "Player tracks": the
`kartcraft_user_tracks` table, under the author's character name, 20 a character), where every
browser's Player tracks (the editor button's right-click menu) lists it for a test lap; its
author takes it down there. A published track is a course on the server too, `pt-<id>`: the
Race Finder's Player tracks tab (each with its good and bad votes, sorted newest first, by
rating or by name) queues for a race on one (by that key; never in "any" or a zone)
or opens its time trial (`R\tTB\tpt-<id>`), with records kept as for any course, lined up on its
own grid where it has one. Q and E, or the middle mouse held, turn the gate about to be placed,
and the mouse wheel widens or narrows it (`gate`, 2 to 45 yards: the server's reach). A
publish carries each gate as `x,y[,o[,w]]` (`o` empty where only the width `w` follows), and so
do the server's `PT` lines, so a test lap or race on a player track has its gates' facings and
widths.
To make a saved track a real course, paste the
object into `courses`, then run `course-gen marshals` and `course-gen pads` for its marshal and
pads, `pack.py`, and redeploy the server.

## Dungeon courses

A course on a dungeon's map (not 0 or 1) is a dungeon course, raced in a copy of the dungeon
(`Kartcraft.cpp` "Race copies"). The Race Finder offers it in its zone like any course: a race
that pops on one gets a fresh copy of its own, and every racer, party or not, is bound to it
(not for keeps) and lands on its grid from wherever they were. A time trial (the track browser,
Try again, the Race Finder's) runs in the player's own copy, kept while it lives, so a run
after a run makes no new copy. A raid's copy wants a raid group, so a raid is never raced. The
Race Marshal stands inside too (Stratholme's in King's Square, past the main gate): the party's
leader (or a player with no party) hosts there, and only the host's party in the host's copy
can join; the grid, the unstuck hop and every checkpoint stay in that copy, as a teleport within
one map does. A player track published on a dungeon's map is a dungeon course too.

In the kart, a driver springs none of Stratholme's position events: the rat-trap gates (and their
rat swarm) and The Unforgiven's ambush on Market Row (`instance_stratholme.cpp`, a hunk of
`kartcraft.patch`). The course's locked King's Square gates open for its race (`doors`).

## Race Marshals

A Race Marshal (a goblin flag girl, creature 190000, script `npc_kartcraft_marshal`) stands
beside every course start. Talking to her lists, for the courses that start there: "Host a
race" per course, "Join <host>'s race" per waiting race, "Start the race" for the host, "Leave
the race" while in a race or the Race Finder's queue, "Open the Race Finder" and "How do races
work?". A choice runs as the addon line it stands for (`R\tH`, `R\tJ`, `R\tS`, `R\tX`), so a
client that still sends those lines does the same; hosting or joining on foot puts the driver
in the kart. "Host a race" asks first whether ground types count (npc_text 190004): "ground
types off" sends `R\tH\t<key>`, "ground types on" `R\tH\t<key>\t1`, and every racer in that race
drives by the host's choice (`R\tGT`, `docs/ctr-weapons-assets-and-net.md` B3); a waiting race
with them on says so in its "Join" line.

- **Placing.** After adding or moving courses, run
  `WOW_DATA=<your WoW>/Data cargo run --release -p course-gen -- marshals`: starts within 40 yards
  of an earlier course's start share its marshal, who stands 10 yards left of the start line
  (across the way from checkpoint 0 to 1), facing it, at the terrain's height there. It writes
  each course's `marshal`; then run `pack.py`.
- **The rows.** `pack.py` writes `setup/vmangos/sql/kartcraft.sql`: the creature template, its
  texts (npc_text/broadcast_text 190000-190004) and one spawn per distinct spot (creature guids
  from 400000; 400000-400999 are reserved). It deletes its own rows first, so it runs any number
  of times.
- **Deploying.** vmangos-deploy runs every `server/storage/database/custom-sql/*.sql` against the
  world database each time the database container starts. Setup copies the file there and, when
  it changed, restarts the database with the world server stopped. By hand: copy it to
  `storage/database/custom-sql/kartcraft.sql`, then `docker compose -p world-of-kartcraft stop
  mangosd`, `... restart database`, `... up -d`. The gossip itself is in the server image, so a
  changed `Kartcraft.cpp` also needs the image rebuilt.
