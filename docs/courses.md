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
- `map`: the map id (0 Eastern Kingdoms, 1 Kalimdor).
- `laps`: 0 for a sprint from the first checkpoint to the last; else laps back through the first.
- `checkpoints`: at least two, WoW world `x`, `y` (yards); the first is the start, where a
  driver hosts or joins. Optional `z` (the server finds the ground itself). Optional `gate`:
  the gate's half-width in yards. The client passes a checkpoint through a gate this wide
  (default 20); the server accepts a report within `max(45, gate)` yards.
- `marshal`: where the course's Race Marshal stands (`x`, `y`, `z`, orientation `o`), written by
  `course-gen marshals` (below); courses that share a start share the spot. Required by `pack.py`.
- `crates`: optional. When present it replaces the layout the server builds along the
  segments: `kind` `F` (fruit crate), `Q` (weapon crate) or `W` (wumpa fruit), `x`, `y`, and an
  optional `z` (else the ground's).
- Up to 1000 courses. The Race Finder offers them by zone, in the order the file first names each
  zone; a queue line names a zone by that place (`z3`), so keep zones' first appearances in order
  when adding courses (the exporter appends).

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

## Race Marshals

A Race Marshal (a goblin flag girl, creature 190000, script `npc_kartcraft_marshal`) stands
beside every course start. Talking to her lists, for the courses that start there: "Host a
race" per course, "Join <host>'s race" per waiting race, "Start the race" for the host, "Leave
the race" while in a race or the Race Finder's queue, "Open the Race Finder" and "How do races
work?". A choice runs as the addon line it stands for (`R\tH`, `R\tJ`, `R\tS`, `R\tX`), so a
client that still sends those lines does the same; hosting or joining on foot puts the driver
in the kart.

- **Placing.** After adding or moving courses, run
  `WOW_DATA=<your WoW>/Data cargo run --release -p course-gen -- marshals`: starts within 40 yards
  of an earlier course's start share its marshal, who stands 10 yards left of the start line
  (across the way from checkpoint 0 to 1), facing it, at the terrain's height there. It writes
  each course's `marshal`; then run `pack.py`.
- **The rows.** `pack.py` writes `setup/vmangos/sql/kartcraft.sql`: the creature template, its two
  texts (npc_text/broadcast_text 190000-190001) and one spawn per distinct spot (creature guids
  from 400000; 400000-400999 are reserved). It deletes its own rows first, so it runs any number
  of times.
- **Deploying.** vmangos-deploy runs every `server/storage/database/custom-sql/*.sql` against the
  world database each time the database container starts. Setup copies the file there and, when
  it changed, restarts the database with the world server stopped. By hand: copy it to
  `storage/database/custom-sql/kartcraft.sql`, then `docker compose -p world-of-kartcraft stop
  mangosd`, `... restart database`, `... up -d`. The gossip itself is in the server image, so a
  changed `Kartcraft.cpp` also needs the image rebuilt.
