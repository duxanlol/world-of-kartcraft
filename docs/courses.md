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
`Kartcraft.cpp`) and refreshes the new-file hunks of `setup/vmangos/kartcraft.patch`, which the
server image is built from; rebuild the server image to take the change. `pack.py --check`
exits non-zero when the header or the patch is stale. The client needs nothing: it embeds the
file at compile time (`crates/benilla-app/src/kart/race.rs`, `COURSES`), so a rebuild picks it up.
The checkpoints a race uses still reach the client from the server at the start (`R\tC`).
