# ctr-weapons: what the crate implements

`crates/ctr-weapons` holds CTR's weapon logic as pure, deterministic Rust. It uses CTR's units:
whole lu, lu per frame, 4096ths of a turn, and 30 Hz frames. CTR's rules are in
`docs/ctr-weapons-spec.md`, and the wire format is in `docs/kartcraft-weapons-protocol.md`. Every
constant in the code cites its line in the decompilation.

## Modules

- **`item`**: `Item`, with CTR's ids (0 Turbo … 11 Missile ×3, 15 none). It also defines shots
  per item, `single()` (×3 → ×1), the juiced rule (`wumpa >= 10` when the item is fired) and
  `WUMPA_MAX`.
- **`roulette`**: `Rng`, CTR's `MixRNG_Scramble` LCG (boot seed 100), and the six weighted
  tables. `roll(rng, position, racers, lap, ctx)` is `VehPhysGeneral_SetHeldItem` for a race.
  The table depends on rank and field size (8, 6, 5, 4, 3 and 2 drivers). A field of any other
  size rolls uniformly over ids 0–11. On lap 1, Race4 is downgraded to Race3. The roll is bucketed
  as `(s >> 3) % 200 * N / 200`, then the overrides run: Spring → Turbo; a second warp orb →
  Missile ×3; with 3 or more humans, a third Missile ×3 holder gets Missile ×1. `roll` takes
  `&RaceItems` and does not change it. The server records the result with `RaceItems::take` and
  frees it with `release`. Adventure-boss, battle and Crystal Challenge rules are left out.
- **`held`**: `Held`, the holder's client-side state machine. A crate starts a 90-frame spin, and
  a Fire press stops the spin once the timer is below 70 (21 frames in). The result comes from
  the server (`set_result`) and shows on the frame after the timer reaches 0. Each ×3 shot sets a
  5-frame gap. After the last shot, the item stays shown for 5 frames (×3) or 30 frames (single)
  and then disappears. While a bomb is out, Fire detonates it; while a shield is up, Fire launches
  it. Call `tick()` first and then `fire()` in the same frame, which is CTR's order.
- **`projectile`**: `Bomb` (also the shield bomb), `Missile`, `WarpOrb` and `Mine` (TNT, Nitro,
  beaker). Each has `step(world, karts) -> Vec<Outcome>`. `Field` holds a race's live weapons and
  enforces the caps: 12 bombs and missiles, and a pool of 10 mines in which the oldest mine goes
  off when an 11th is added. It also handles weapons meeting each other: a bomb or missile
  explodes on a mine, a bomb and a missile destroy each other, and a bomb or missile blast sets
  off the mines and missiles inside it. A mine or missile set off this way does no damage.

## Faithful

- Bomb speeds are 96 lu/frame forward and 28 backward. A bomb turns 4 units per frame toward its
  lock-on target, and the second Fire press detonates it at the end of its next tick. The blast
  radius is 320 lu, or 512 juiced, and never hurts the owner.
- Missile speeds are 80 or 96 lu/frame, and missiles turn 64 or 128 units per frame. They have no
  lifetime. A missile whose target leaves keeps flying straight. Its blast radius is 128 lu.
- Contact tests follow `RB_Hazard_CollideWithDrivers`: a cylinder of XZ < r and |dy| < 2r for
  bombs, missiles and orbs, and a sphere for mines. The radii are 96, 192, 120 and 80 lu. The
  owner is spared for 60 frames (bomb, missile) or 10 (orb, shield bomb, mine).
- Grounded weapons ride 48 lu above the ground probe. Gravity is 4 lu/frame² (8 for missiles),
  up to a terminal speed of 96. A weapon with no ground within 2304 lu below explodes.
- Mines:
  - Driving into a TNT puts it on your head.
  - If you already carry a TNT or are squished, you are blasted instead, and the TNT blows up
    (320 lu, owner included).
  - With a mask or shield, the TNT only blows up.
  - Nitro blasts you and blows up with a 128 lu radius to players.
  - A beaker spins you out; a red one also adds the rain cloud.
- An `Outcome::Hit` is a damage attempt. The victim's own kart applies the mask, shield and
  invincibility checks, as `VehPickState_NewState` does.

## Approximated

- **Walls.** CTR has no geometric wall test. Weapons explode on "weapon react" trigger quads and
  level hitboxes. WoW terrain has neither, so a wall is any face steeper than `GROUND_NORMAL_Y`
  (about 50°) that a horizontal ray crosses at the weapon's centre height, from its old position
  to its new one. Ground faces such as ramps let the ray pass, and the ground probe lifts the
  weapon onto them. A bomb or missile that hits a wall steps back one frame and explodes, as it
  does on a trigger. Two consequences:
  - Steps lower than 48 lu are climbed.
  - Steep natural slopes count as walls.
- **The ground** comes from `Collision::ground` (the first ground face down from 256 lu above),
  not from CTR's quadblock BSP.
- **The warp orb** has no track rail: there is no checkpoint polyline. It always homes, using
  CTR's homing-mode turn rates (0x100, up to 0x400 inside 768 lu, 0x40 during its safety
  frames), 112 lu/frame and the ±4 vertical approach. It ignores walls (CTR's rail never meets
  them) and stays at least 48 lu above the ground.
  - Its target is fixed at launch (the server picks the leader) and is not refreshed each frame.
  - It never blasts the same kart twice.
  - When juiced, it marks the victim and everyone behind as hit, then retargets the closest kart
    ahead still unhit, by rank.
  - It dies when it hits its target (unless juiced) or the leader, or when it has no target.
    There is no 120-frame turnaround.
- **Motion** uses `f32` trigonometry, not CTR's 4096-entry fixed-point tables, so headings agree
  only to float precision. Velocities are lu per frame without the `ms >> 5` scaling, because a
  frame is always 32 ms.
- **Not modelled:**
  - the missile's 10-frame TNT decoy (`framesSeekTargetTnt`);
  - beaker throwing (Up held);
  - weapons breaking crates;
  - bot weapon AI;
  - Clock, Turbo, Mask and Shield effects on karts (`ctr-kart`'s job).
- **Battle-only items** (Invisibility, Super Engine) have ids and the BattleDefault table, but
  there are no battle rules.

## Tables for the server

`cargo test -p ctr-weapons print_tables -- --nocapture` prints the tables as C++ arrays and their
exact odds over the LCG's full period. Race4 stores 20 entries but samples only the first 19.

```
static const uint8 kRace1[20] = { 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 1, 1, 0, 6 };
static const uint8 kRace2[52] = { 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 1, 1, 1, 1, 1, 10, 10, 10, 0, 0, 0, 0, 0, 6, 6, 6, 2, 2, 2, 11, 11, 9, 9, 9, 9, 9, 7, 7, 7, 7, 7, 7, 7, 8 };
static const uint8 kRace3[20] = { 4, 3, 1, 10, 10, 0, 0, 6, 2, 2, 11, 9, 9, 9, 7, 7, 7, 7, 7, 8 };
static const uint8 kRace4[19] = { 0, 6, 2, 11, 9, 9, 9, 9, 9, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8 };
static const uint8 kBossRace[20] = { 4, 3, 1, 10, 10, 0, 0, 6, 2, 11, 9, 9, 9, 7, 7, 7, 7, 7, 8, 8 };
static const uint8 kBattleDefault[20] = { 4, 3, 1, 1, 1, 10, 10, 6, 6, 2, 2, 2, 2, 2, 7, 7, 7, 13, 13, 12 };
```

Picking a table by `rank` (0 = 1st) and field size:

| Field size | Ranks → table |
|---|---|
| 8 | 0 Race1; 1–3 Race2; 4–5 Race3; 6–7 Race4 |
| 6 | 0 Race1; 1–2 Race2; 3–4 Race3; 5 Race4 |
| 5 | 0–3 Race1–Race4; 4 Race4 |
| 4 | 0–3 Race1–Race4 |
| 3 | 0 Race1; 1 Race2 if `scramble() & 1` else Race3; 2 Race4 |
| 2 | 0 Race1; 1 BossRace |
| other | `scramble() % 12`, drawn after the bucket draw |
