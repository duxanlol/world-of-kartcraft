# Crash Team Racing (PS1, NTSC-U) weapons, crates and Wumpa: port spec

Source of truth: the decompiled game, a checkout of [CTR-native](https://github.com/CTR-tools/ctr-native)
(`C:\Users\user\playground\ctr-native-ref`). All `file:line` references are relative to its `game/`
directory unless they start with `include/`. Where the repo has both a retail and a native path,
the **native C path is quoted**.

This is the companion to `docs/ctr-physics-spec.md` and uses the same conventions:

| Quantity | Unit | Notes |
|---|---|---|
| Frame | 30 Hz; `elapsedTimeMS` nominal **32 CTR-ms** | 960 CTR-ms = 1 s. Timers named `*MS`, `*Timer` that subtract `elapsedTimeMS` are in CTR-ms; divide by 32 for frames. |
| Driver position / speed | 1/256 lu, 1/256 lu per frame | `posCurr`, `speed`, `velocity` (physics spec §0). |
| **Weapon position / velocity** | **whole lu, lu per frame** | Weapons move `inst->matrix.t` (lu) by `vel * ms >> 5` (`231/RB_MovingExplosive.c:199-201`). So a weapon velocity of 96 is 96 lu/frame = 24576 in driver speed units. Crash's top speed 13140 is about 51 lu/frame. |
| Angles | 4096 = 360° | |
| Hit "radius" constants | **squared lu** | `RB_Hazard_CollideWithDrivers(..., hitRadius, ...)` compares `dx²+dz²` (or `dx²+dy²+dz²`) against the constant (`231/RB_Hazard.c:60-68`). 0x2400 = 9216 → 96 lu. |

Some counters tick **once per frame** (not by ms). These are marked "frames" below.

The item IDs (`driver->heldItemID`, `include/namespace_Vehicle.h:641-657`):

| ID | Item | ID | Item |
|---|---|---|---|
| 0 | Turbo | 8 | Clock (N. Tropy) |
| 1 | Bomb ×1 | 9 | Warp orb (warpball) |
| 2 | Missile ×1 (also the shared "bomb/missile" fire ID) | 10 (0xA) | Bomb ×3 |
| 3 | TNT (Nitro when juiced) | 11 (0xB) | Missile ×3 |
| 4 | Beaker / potion | 12 (0xC) | Invisibility (battle only) |
| 5 | Spring (unused; always replaced by Turbo) | 13 (0xD) | Super Engine (battle only) |
| 6 | Power shield | 15 (0xF) | none |
| 7 | Aku Aku / Uka Uka mask | 16 (0x10) | roulette spinning |

**Juiced** = `numWumpas >= 10` (`DRIVER_WUMPA_JUICED_COUNT`, `include/namespace_Vehicle.h:668`), tested **at the moment the item is fired**, not when it was picked up.

---

## 1. Weapon crates ("?" boxes)

### 1.1 Collision

Crates are level instances (`PU_RANDOM_CRATE`) whose collision is a BSP hitbox in the level file, not a radius in code. The kart's swept 25 lu sphere (physics spec §9.1) touches the hitbox and `COLL_MOVED_PlayerSearch` calls the model's `LInC` callback (`COLL.c:2186-2236`, `2337-2343`). Crates use a pass-through hitbox class, so the kart is not deflected.

The callback is `RB_CrateWeapon_LInC` → `RB_CrateWeapon_ThCollide` (`231/RB_Crate.c:107-271`), registered in `zGlobal_DATA.c:1114`. A crate can also be broken by:
- a moving bomb, missile or fired shield (the item goes to the shooter if the shooter is human, `RB_Crate.c:152-164`);
- an explosion's blast sphere (`231/RB_Burst.c:153-192`).

**Port contract:** give each crate an AABB or sphere hitbox and test it against the kart sphere and against projectiles.

### 1.2 Break, then respawn

| Step | Value | Cite |
|---|---|---|
| Hit accepted when | `cooldown == 0` and scale is exactly 0 or exactly 0x1000 | `RB_Crate.c:123` |
| On hit | `cooldown = 30` **frames**. If the crate was full size: scale → 0, spawn break-up effect, SFX 0x3C | `:125-150` |
| Respawn | `cooldown` drops by 1 per frame (paused while `boolPauseCooldown`). Then scale grows +0x100 per frame from 0 to 0x1000 (16 frames) | `RB_Crate.c:48-75` |
| **Total respawn** | **30 + 16 = 46 frames ≈ 1.53 s** | |
| Touching it at scale 0, cooldown 0 | restarts the 30-frame cooldown and gives no item (a kart parked in the slot delays the respawn) | `:123-127` |
| A dropped TNT, Nitro or beaker sitting on the crate | pauses the cooldown until that mine is set off | `:230-240`, `RB_Hazard.c:165-180`, `RB_GenericMine.c:166-170` |

### 1.3 Who gets the roulette

After the crate breaks, these checks run in order. If any one fails, the crate is **still consumed**, but there is no roulette (`RB_Crate.c:152-226`):

1. The hitter is a player kart, or a bomb, missile or shield owned by a **human**. Bots get nothing from crates; see §8.
2. `heldItemID == NONE`, or the "item is ending" cooldown `noItemTimer` is running. So **a kart that already holds a weapon gets nothing.**
3. `numHeldItems == 0` (no stacked ammo left).
4. No fire request is pending this frame.
5. No red-potion rain cloud with the `ITEM_ROLL` effect.
6. `clockReceive == 0`. A kart under the Clock gets nothing.

When all pass:
- `heldItemID = ROULETTE`, `itemRollTimer = 90`;
- start the looping roulette SFX **0x5D** if it is not already playing (global flag `ROLLING_ITEM`);
- `noItemTimer = 0`.

### 1.4 Roulette spin

| | Value | Cite |
|---|---|---|
| Duration | `itemRollTimer` 90 → 0, **1 per frame = 3.0 s** | `RB_Crate.c:198`, `Vehicle/VehPhysProc.c:580-589` |
| Early stop | Circle tapped while `itemRollTimer < 70` (at least 20 frames spun) sets it to 0, so the result comes on the next frame. Earlier taps are ignored. | `VehPhysProc.c:833-835`, `896-902` (`ITEM_ROLL_FAST_STOP_FRAMES = 70`, `:13`) |
| Result | `VehPhysGeneral_SetHeldItem` (§1.5) when the timer reaches 0 | `VehPhysProc.c:592` |
| Result SFX | **0x5E** normally, **0x41** when juiced | `VehPhysProc.c:594-606` |
| Loop SFX stop | 0x5D stops once no player's roll is running | `UI/UI_RenderFrame.c:1041-1059` |
| HUD icon while spinning | a random icon each frame (`rand()`). **Cosmetic only; the real pick happens at the end.** | `UI/UI_Weapon.c:113-140` |

### 1.5 Result: the weighted item tables (`VehPhysGeneral_SetHeldItem`, `Vehicle/VehPhysGeneral.c:1651-1962`)

**It is random.** The roll is `rng = (MixRNG_Scramble() >> 3) % 200` and `item = table[rng * N / 200]` (`:1807-1841`). `MixRNG_Scramble` is a 16-bit LCG, `s = (s*0x6255 + 0x3619) & 0xFFFF` (`MixRNG/MixRNG.c:3-10`).

**Picking the table.** `mode = numPlyrCurrGame + numBotsNextGame`, i.e. the drivers on the field (`:1692`). `rank` is `driverRank`, 0 = 1st.

| Situation | Table |
|---|---|
| Battle, default weapon set | BattleDefault (`:1700-1703`) |
| Battle, custom weapon set | uniform over the chosen weapons (`battleSetup.RNG_itemSetCustom`, `:1845-1847`) |
| Crystal Challenge | always Bomb ×1; Turbo on Skull Rock and Rampage Ruins (`:1849-1859`) |
| mode 8 (1P Arcade/Adventure, 8 karts) | rank 0 → Race1; 1, 2, 3 → Race2; 4, 5 → Race3; 6, 7 → Race4 (`:1719-1728`) |
| mode 6 (2P + 4 bots) | rank 0 → Race1; 1, 2 → Race2; 3, 4 → Race3; 5 → Race4 (`:1731-1743`) |
| mode 5 | rank 0-3 → Race1-Race4; rank 4 → Race4 (`:1745-1751`) |
| mode 4 (4P VS) | rank 0-3 → Race1-Race4 (`:1753-1755`) |
| mode 3 (3P VS) | rank 0 → Race1; rank 1 → Race3 or Race2 (50/50 on `rng & 1`); rank 2 → Race4 (`:1758-1773`) |
| mode 2 (boss race, also 2P VS with no bots) | rank 0 → Race1, otherwise BossRace (`:1785-1791`) |
| any other count | `MixRNG % 12`, uniform over IDs 0-11 (`:1862-1866`) |
| Race4 picked on **lap 1** (`lapIndex == 0`) | downgraded to Race3 (`:1801-1804`) |

**Tables** (`zGlobal_DATA.c:8108-8174`). The probabilities below were computed exactly over the LCG's full period. Note that the `*N/200` bucketing makes them not quite equal per entry.

| Item | Race1 (N=20) | Race2 (N=52) | Race3 (20) | Race4 (19 of 20) | BossRace (20) | BattleDefault (20) |
|---|---|---|---|---|---|---|
| Beaker | 40.0% | 19.5% | 5.0% | – | 5.0% | 5.0% |
| TNT | 40.0% | 15.5% | 5.0% | – | 5.0% | 5.0% |
| Bomb ×1 | 10.0% | 9.5% | 5.0% | – | 5.0% | 15.0% |
| Bomb ×3 | – | 5.5% | 10.0% | – | 10.0% | 10.0% |
| Turbo | 5.0% | 10.0% | 10.0% | 5.5% | 10.0% | – |
| Shield | 4.9% | 5.5% | 5.0% | 5.5% | 5.0% | 10.0% |
| Missile ×1 | – | 6.0% | 10.0% | 5.0% | 5.0% | 25.0% |
| Missile ×3 | – | 3.5% | 5.0% | 5.5% | 5.0% | – |
| Warp orb | – | 10.0% | 15.0% | 26.0% | 15.0% | – |
| Mask | – | 13.5% | 25.0% | 42.0% | 25.0% | 15.0% |
| Clock | – | 1.5% | 4.9% | 10.4% | 9.9% | – |
| Super Engine / Invisibility | – | – | – | – | – | 10.0% / 4.9% |

Race4 is stored as 20 bytes, but retail samples only the first 19 (`VehPhysGeneral.c:1827-1828`).

**Post-roll overrides, applied in this order:**

1. **Adventure boss race** (`:1871-1905`), by `timesLostBossRace`:
   - fewer than 3 losses: Mask, Clock or Warp orb → Missile ×3;
   - 3 losses: Mask or Clock → Missile ×3;
   - 4 losses: Clock → Missile ×3.
   - On Dragon Mines (Komodo Joe), Missile ×3 → Missile ×1.
2. Spring → Turbo (`:1907-1911`).
3. **One warp orb at a time.** If the global `WARPBALL_HELD` is set, a Warp orb becomes Missile ×3. Otherwise the flag is set. It clears when the orb dies (`:1913-1929`, `231/RB_Warpball.c:33`).
4. **Missile ×3 cap.** With 3 or more human players and not in battle, at most 2 drivers may hold Missile ×3 at once (`numPlayersWith3Missiles`). A third gets Missile ×1 (`:1932-1953`). The count is released when the item runs out (`VehPhysProc.c:622-629`).
5. Bomb ×3 and Missile ×3 set `numHeldItems = 3` (`:1956-1959`).

### 1.6 Firing, ammo, item removal (`Vehicle/VehPhysProc.c:804-893`)

Circle **tap** is honoured only when **all** of these hold:
- `kartState` is NORMAL, DRIFTING or ANTIVSHIFT (so not spinning or blasted);
- no TNT on your head;
- no roulette running;
- `noItemTimer == 0`;
- no rain cloud with `ITEM_ROLL`;
- no `clockReceive`.

Two special cases come first:
- If you have a live **bomb** out, the tap detonates it (`:812-819`).
- If you have a **shield** up, the tap fires it (`:822-829`).

Neither uses ammo or a roulette check.

The tap then sets `ACTION_WEAPON_FIRE_REQUEST` and handles ammo:
- `numHeldItems > 0`: decrement it and set `noItemTimer = 5` frames. This is the gap between stacked shots.
- `numHeldItems == 0`: `noItemTimer = 30` frames.

When `noItemTimer` reaches 1 with `numHeldItems == 0`, the item becomes NONE (`:615-636`); the HUD icon flickers meanwhile (`UI_Weapon.c:85-89`). So a ×3 item gives exactly 3 shots.

The shot itself happens at the start of the **next** frame. `MainFrame.c:301-306` calls `VehPickupItem_ShootOnCirclePress` before the driver stages, and it maps Bomb ×1/×3 and Missile ×3 onto shared ID 2 (`VehPickupItem.c:1537-1569`). The same function also applies kart-collision damage queued last frame (`pendingDamageType`, `:1541-1544`).

---

## 2. Wumpa fruit

| | Value | Cite |
|---|---|---|
| Count | `numWumpas` s8, clamped to `[0, 10]` | `231/RB_Player.c:141-197` |
| Single fruit | Picked up by players and bots: SFX 0x43; the HUD queue (`PickupWumpaHUD.numCollected`) +1 for players. **No respawn**: scale set to 0, and `LInC` ignores scale 0 (`RB_Fruit.c:75-112`). | `231/RB_Fruit.c:20-66` |
| Fruit crate (`PU_FRUIT_CRATE`) | Same 30 + 16-frame break and respawn as weapon crates. Gives `MixRNG % 4 + 5` = **5-8 fruit**. The queue is **assigned**, not added (a pending queue is overwritten). Can also be broken by your bomb, missile or shield. | `231/RB_Crate.c:273-372` |
| Credit to the counter | One fruit every 5 frames, after the icon flies to the HUD: +1 each time (`RB_Player_ModifyWumpa`), SFX **0x42** | `UI/UI_RenderFrame.c:212-257` |
| Reaching 10 | SFX **0x41** ("juiced up") and the HUD flash | `RB_Player.c:188-197` |
| Speed effect | `min(numWumpas, 9)` gives up to +269 top speed. Already in physics spec §3.2 / §7.4. | `Vehicle/VehPhysGeneral.c:1998-2057` |

**Fruit lost** (`RB_Player_ModifyWumpa(d, -n)`, ignored while the mask is active, `RB_Player.c:153-161`):

| Event | Lost | Cite |
|---|---|---|
| Spin-out (beaker, clock, squish, burn, hazards) | 1 | `VehPhysProc.c:2868-2871` (`SpinFirst_Init`) |
| Blasted (bomb, missile, TNT, Nitro, warp orb, mask or shield ram) | **3** | `Vehicle/VehStuckProc.c:1466-1469` (`VEH_TUMBLE_WUMPA_PENALTY = 3`, `:65`) |
| Mask-grab (fell off the track), plant-eaten | 2 | `VehStuckProc.c:650`, `903` |

**What juiced changes**, decided when firing:

| Item | Normal | Juiced |
|---|---|---|
| Turbo | fire level 128 (cap +3072) | 256 (cap +4096) |
| Bomb | blast radius 320 lu | **512 lu** |
| Missile | 80 lu/frame, turn 64 per frame | 96 lu/frame, turn 128 per frame (blast radius unchanged, 128) |
| TNT | TNT crate | **Nitro** |
| Beaker | green (spin) | **red** (spin + rain cloud) |
| Shield | green, 12 s | **blue, no time limit** |
| Mask / Clock / Invisibility / Super Engine | 7680 ms (8 s) | 11520 ms (12 s) |
| Warp orb | hits the leader, then dies | hits every driver ahead, then dies on the leader |

---

## 3. Weapons

All weapons are spawned in `VehPickupItem_ShootNow` (`Vehicle/VehPickupItem.c:579-1535`). They spawn at the kart's own instance matrix (position and yaw).

### 3.1 Projectile collision (shared)

| Projectile | vs driver | vs mine (TNT/Nitro/beaker) | Cite |
|---|---|---|---|
| Bomb, missile, fired shield | XZ distance < **96 lu** (0x2400) and \|dy\| < 192 | 3D distance < 96 lu | `231/RB_MovingExplosive.c:325-341`, `RB_Hazard.c:25-85` |
| Warp orb | XZ < **192 lu** (0x9000), \|dy\| < 384 | 3D < 96 lu | `RB_Warpball.c:701-783` |
| TNT / Nitro (placed) | 3D distance < **120 lu** (0x3840), \|dy\| < 240 | – | `231/RB_GenericMine.c:154` |
| Beaker (placed) | 3D distance < **80 lu** (0x1900), \|dy\| < 160 | – | same |

**Owner safety.** The owner is ignored by its own projectile's contact test while `parentSafetyFrames > 0`, which counts down 1 per frame:

| Projectile | Safety frames | Cite |
|---|---|---|
| Bomb, missile | 60 | `VehPickupItem.c:849` |
| Warp orb, shield bomb | 10 | `:1108`, `231/RB_MaskShieldCloud.c:455` |
| Mines | 10 | `VehPickupItem.c:1189`, `1362` |

**Ground and walls.** Projectiles probe only a vertical segment around themselves (`RB_MovingExplosive.c:220-323`).
- On ground, the projectile snaps to `ground + 48 lu`.
- With no ground under it, it falls (gravity 4 lu/frame², missile 8, terminal −96).
- With no ground within 2304 lu below, it explodes.

There is **no geometric wall test**. Bombs, missiles and beakers explode, and warp orbs turn around, on (a) trigger quads flagged "weapon react" (`COLL_STEP_TRIGGER_WEAPON_REACT`, `include/namespace_Coll.h:33`) or (b) level hitbox instances. **Port contract:** treat steep (wall) faces in the projectile sweep as weapon-react.

**Global limit.** At most **12 bombs + missiles** in flight (`numMissiles`, `VehPickupItem.c:641-648`); further shots are silently dropped.

### 3.2 Bowling bomb (IDs 1, 10; model `DYNAMIC_BOMB`)

| | Value | Cite |
|---|---|---|
| Launch | forward `vel = facing * 3 >> 7` = **96 lu/frame** | `VehPickupItem.c:822-823` |
| Backward | Down held at fire time (or bot flag): `vel = -((vel>>1)*3/5)` = **−28 lu/frame** | `:829-835` |
| Steering | target = missile lock-on (§3.3). Turns toward it at **4 angle units/frame** (0.35°), so almost straight; speed held at 96. A backward bomb's velocity is negated, so it runs away from that target. | `RB_MovingExplosive.c:129-143` |
| Roll | visual `dir.x += 0x200` per frame (backward: −) | `:204-218` |
| Remote detonation | Circle while the bomb is out sets `flags |= 2`; it explodes at the end of its next tick. One bomb at a time (`d->instBombThrow`). | `VehPhysProc.c:812-819`, `RB_MovingExplosive.c:357-362` |
| Also explodes on | driver contact, mine contact (sets the mine off), **missile** contact (both die; warp orbs are ignored), wall/trigger, no ground | `RB_MovingExplosive.c:243-362` |
| Lifetime | none | |
| Blast | sphere **320 lu** (0x19000); juiced **512 lu** (0x40000). Victims are **blasted**. The shooter is always excluded. Also sets off mines, missiles and crates in range. | `231/RB_Burst.c:286-317` |
| SFX | launch **0x47**, roll loop **0x48**, explode **0x49**; voice 10 | `VehPickupItem.c:786-790`, `RB_MovingExplosive.c:50`, `379` |

### 3.3 Tracking missile (IDs 2, 11; model `DYNAMIC_ROCKET`)

**Lock-on target** (`VehPickupItem_MissileGetTargetDriver`, `VehPickupItem.c:353-506`). Candidates exclude:
- self;
- mask-grabbed drivers;
- teammates in battle;
- invisible drivers.

Each candidate is projected through the shooter's view:
- a human uses its camera's `matrix_ViewProj`;
- a bot uses a matrix built from its kart rotation.

The candidate must land inside the viewport rect, inset 31/30 px left/right and 21/20 px top/bottom. Of those, the closest in XZ wins.

Fallbacks when no one is on screen (`:654-733`):
- **Battle:** the closest valid driver anywhere.
- **Race, shooter is a bot:** the driver one place ahead (`rank-1`), but only on odd `elapsedEventTime`.
- **Race, shooter is human:** no target; the missile flies straight.

| | Value | Cite |
|---|---|---|
| Speed | **80 lu/frame** (`5 >> 8`); juiced **96** (`3 >> 7`) | `VehPickupItem.c:837-847`, `RB_MovingExplosive.c:149-163` |
| Homing | Each frame, aim yaw = `ratan2(target − missile)` in XZ. Yaw moves toward it by at most **64 units/frame** (5.6°, 169°/s); juiced **128** (11.25°). Velocity is rebuilt from the new yaw, so the missile always flies along its yaw. | `RB_MovingExplosive.c:121-171`, `RB_Hazard.c:284-313` |
| Target lost | the target turns invisible → `driverTarget = NULL`, and the missile continues straight | `:61-72` |
| TNT decoy | if the target dropped a mine last frame, then for **10 frames** the missile steers at the target's dropped TNT (`instTntSend`) instead | `:75-118` |
| Lifetime | **none**: it flies until it hits something | (`timeAlive` is a stat only, `RB_Burst.c:103-106`) |
| Blast | sphere **128 lu** (0x4000) in both normal and juiced; victims **blasted**, shooter excluded | `RB_Burst.c:291-295` |
| Exhaust particles | single-player only | `:186-197` |
| Victim HUD | lock-on reticle (`ACTION_TRACKER_TARGETED`) | `UI/UI_RaceHud.c:240-330` |
| SFX | launch **0x4A**, flight loop **0x4B**, explode **0x4C**; voice 11 | `VehPickupItem.c:808-812`, `RB_MovingExplosive.c:43`, `392` |

### 3.4 TNT and Nitro (ID 3)

The item is placed on the ground at the kart's position, behind the moving kart:
- model TNT (`STATIC_CRATE_TNT`), or **Nitro** (`PU_EXPLOSIVE_CRATE`) when juiced (`VehPickupItem.c:1143-1150`);
- scale grows from 0 by +0x200/frame (8 frames to full, `RB_GenericMine.c:139-152`);
- it is placed in the shared mine pool (§3.9);
- drop SFX **0x52**, voice 15.

Human TNT is never thrown; bosses throw it with velocity flags (`VehPickupItem.c:1197`, `PotionThrow`).

**Driver touches a Nitro.** The victim is **blasted** (`RB_GenericMine.c:249-258`). The Nitro breaks with glass SFX **0x3F** and an explosion (`RB_Blowup_Init`):
- player damage radius **128 lu**;
- radius vs bots and mines **320 lu** (boss race 256);
- the owner is *not* excluded.

(`231/RB_Blowup.c:131-162`)

**Driver touches a TNT** (`RB_GenericMine.c:261-366`):
- **Victim protected** (mask, invincible or shield): the TNT simply explodes, and a shield pops.
- **Victim already has a TNT on the head, or is squished:** **blasted**, and the old TNT is removed (`:211-247`).
- **Otherwise the TNT jumps onto the head.** Bounce SFX **0x50**, vy = 48 (`:263-301`). It lands at a per-character head height (table `231/RB_TNT.c:78`); landing SFX **0x51**.

**Shake-off count.** On landing, `jumpsRemaining = 8 - (MixRNG % 2)`, so 7 or 8 (`RB_TNT.c:286`). Each **hop start** (`ACTION_JUMP_STARTED`) with `jumpsRemaining != 0` decrements it. A hop when it is already 0 throws the TNT off (`:176-191`). **So 8 or 9 hops are needed** (50/50).
- The thrown-off TNT flies up with vy 48 and explodes on landing (`RB_TNT_ThTick_ThrowOffHead`, `:5-66`).
- A bot sheds it with probability **1/270 per frame** (`:161-173`).

**Fuse.** `numFramesOnHead` counts **90 frames (3.0 s)**:
- "honk" SFX **0x3E** at frames 0, 20, 40, 60, 70, 80;
- at 90: the carrier is **blasted**, explode SFX **0x3D**, blast sphere **320 lu** with the owner not excluded (`RB_TNT.c:194-239`, `RB_Blowup.c:131-157`).

**Early removal.** The TNT on your head explodes immediately if you start spinning, crashing, get mask-grabbed or get blasted (`RB_TNT.c:122-159`).

**While carrying it:**
- top speed −`DamagedSpeed/2` = −2048 (physics spec §3.2; `VehPhysGeneral.c:2084-2091`);
- you **cannot fire** (`VehPhysProc.c:809`).

### 3.5 Beaker / potion (ID 4)

| | Value | Cite |
|---|---|---|
| Colour | green normally; **red** when juiced (`MINE_WEAPON_FLAG_RED_BEAKER`) | `VehPickupItem.c:1312-1334` |
| Default | dropped at the kart, like TNT (grows from scale 0) | `:1389-1477` |
| Throw forward | Up held at fire time: `vel.xz = facing * 15 >> 9` = **120 lu/frame**, `vel.y = 48` | `:1377-1387`, `508-533` |
| Throw backward | bot/boss flag only: **−120 lu/frame**, vy 48 | `:534-549` |
| Throw random | boss: −16..+15 lu/frame, vy 48 | `:550-571` |
| In flight | gravity 4/frame², terminal −96. After **240 CTR-ms (7.5 frames)** it settles on the first ground contact. It breaks on a weapon-react trigger, a hitbox or no ground. | `231/RB_Potion.c:42-191` |
| On landing | `cooldown = 3840 ms` (120 frames); a hit during it counts as a "moving potion" (stat/reason 4) | `RB_Potion.c:149-161`, `RB_GenericMine.c:177` |
| Hit | **spin-out** (damage type 1). Red adds a **rain cloud** (§3.5.1). Glass SFX **0x3F**. | `RB_GenericMine.c:172-206` |

#### 3.5.1 Rain cloud (`RB_RainCloud_*`, `231/RB_MaskShieldCloud.c:466-641`)

- **Duration:** 7680 ms (240 frames).
- **First hit:** effect `ITEM_ROLL` if the victim holds an item. That item is turned back into a roulette that stays 5 frames from finishing until the cloud ends, so the victim can't fire; it re-rolls at the end. Otherwise the effect is `SLOW` (top speed −4096).
- **Another red potion:** refreshes the 7680 ms and re-rolls the effect: `(rng%400)/100` gives 0 SLOW, 1 ITEM_ROLL, 2 ICE_TERRAIN, 3 JUMP_LOCKOUT.
- **Removed** by the mask.

### 3.6 Power shield (ID 6)

| | Value | Cite |
|---|---|---|
| Activate | bubble around the kart, SFX **0x57**; green, or **blue** when juiced | `VehPickupItem.c:910-988` |
| Duration | green `0x2D00` = 11520, minus **32 per frame** → **360 frames (12 s)**, fading over the last 60. Blue: **no timeout**. | `VehPickupItem.c:976-983`, `RB_MaskShieldCloud.c:374-390` |
| Hits absorbed | **1**. Any damage pops it (`SHIELD_FLAG_POP_ON_DAMAGE`), gives `invincibleTimer = 672 ms` (**21 frames**), and the victim laughs. | `Vehicle/VehPickState.c:82-114` |
| Ramming | shielded kart hits an unshielded one: victim **blasted**, shield consumed | `Vehicle/VehPhysCrash.c:215-235` |
| Fire | Circle again: a projectile ("shield bomb") moving **straight** at `facing*3>>7` = **96 lu/frame**, no homing. Explodes like a bomb (blast **320 lu**, victims **blasted**, reason "bomb"). Fire SFX **0x58**, roll loop **0x59**, voice 13. | `VehPhysProc.c:822-829`, `RB_MaskShieldCloud.c:409-463`, `RB_MovingExplosive.c:54-57` |
| Ends also when | race finished, or mask-grabbed (pop SFX **0x58**) | `RB_MaskShieldCloud.c:393-408` |

### 3.7 Turbo (ID 0) and Super Engine (ID 13, battle only)

| | Value | Cite |
|---|---|---|
| Turbo | `VehFire_Increment(d, 2400, FREEZE_RESERVES_ON_TURBO_PAD \| TURBO_ITEM, 128)`, so **2400 ms (75 frames) of reserves** with speed cap +3072. Juiced: fire **256**, cap **+4096**. | `VehPickupItem.c:605-619`; mechanics in physics spec §7 |
| Turbo ram | while boosting from the turbo item (`ACTION_TURBO_ITEM`), hitting a kart with impact > 0xA00 that has no reserves **squishes** it (damage type 3), and the rammer gets a forced high jump | `VehPhysCrash.c:239-246` |
| Super Engine | `superEngineTimer` 7680 / juiced 11520 ms. While it runs and you are gassing forward: every frame, 120 ms of reserves at fire 128 / juiced 256. | `VehPickupItem.c:1481-1495`, `VehPhysProc.c:1286-1307` |

### 3.8 Aku Aku / Uka Uka mask (ID 7)

| | Value | Cite |
|---|---|---|
| Duration | **7680 ms (240 frames, 8 s)**; juiced **11520 (12 s)**. Picking up another mask while one is active refreshes the duration. | `VehPickupItem.c:193-351`, `include/ovr_231.h:51-52`, `RB_MaskShieldCloud.c:190-202` |
| Aku vs Uka | Aku for Crash, Coco, Polar, Pura, Penta; Uka for the rest (`MaskBoolGoodGuy`, `VehPickupItem.c:178-190`). SFX **0x53** (Aku) / **0x54** (Uka). Also switches the music. | `:276-319` |
| Flag | `ACTION_MASK_WEAPON`, set every frame while a mask child thread exists | `VehPhysProc.c:719-764` |
| Invincible | every damage attempt returns 0 (victim laughs, voice 2) | `VehPickState.c:70-80` |
| Speed bonus | +`MaskSpeed` = **+3900** on the target speed (physics spec §3.2) | `VehPhysGeneral.c:2059-2062` |
| Other perks | no turn-speed penalty (`VehPhysGeneral.c:1109`), no terrain accel slowdown (`:1249`), no mud friction (`VehPhysForce.c:683`), no Wumpa loss (`RB_Player.c:153-161`), removes the rain cloud (`RB_MaskShieldCloud.c:527-532`) | |
| Destroys | ramming karts **blasts** them (`VehPhysCrash.c:201-213`). Driving into TNT, Nitro or beakers sets them off with no harm (the TNT path in `RB_GenericMine.c:362-365`). Bombs, missiles and warp orbs still explode on contact but do no damage. | |
| End | fade-out then death (`RB_MaskWeapon_FadeAway`, `RB_MaskShieldCloud.c:19-81`) | |

### 3.9 Mine pool

Placed TNT, Nitro and beakers share a pool of **10** in races; 40 in Crystal Challenge; 7 for the Roo Tubes boss; 3 for the Dragon Mines boss (`231/RB_MinePool.c:9-35`). Adding one when the pool is full sets off the **oldest** (`:76-94`, `RB_GenericMine.c:375-418`).

### 3.10 Clock (ID 8)

| | Value | Cite |
|---|---|---|
| Effect | every other driver gets `RB_Hazard_HurtDriver(type 1)`, a **spin-out**. If that is not blocked (shield, mask, invincible), it also gets `clockReceive = 7680 ms (240 frames)`; juiced **11520 (360)**. User: `clockSend = 30` frames (screen effect). | `VehPickupItem.c:855-908` |
| While `clockReceive` | top speed −`DamagedSpeed*(20-rank)>>4`: −5120 for 1st down to −3328 for 8th, so it hurts the leader most (physics spec §3.2, `VehPhysGeneral.c:2098-2105`). Cannot fire (`VehPhysProc.c:861`). Crates give nothing (`RB_Crate.c:194`). Controller "waddle" rumble (`VehPhysProc.c:443-517`). Cleared when you finish. | |
| Not a time freeze | others keep driving; it is a spin plus a slow | |
| SFX | **0x44**; voice 14 | |

### 3.11 Warp orb (ID 9, `231/RB_Warpball.c`)

**Spawn** (`VehPickupItem.c:1000-1122`):
- at the kart; initial speed `facing*7>>8` = **112 lu/frame**;
- the user is marked "already hit" so it can never hurt them;
- SFX **0x4D**, flight loop **0x4E**, voice 12.

**Target.**
- Normal: the first driver in race order who is not the user and has not finished, i.e. **the leader**, or 2nd if the user leads (`RB_Warpball_GetDriverTarget`, `:127-198`).
- Fired from 1st place: no target, so it turns around and dies.
- Juiced: the nearest driver **ahead along the track** that it has not already hit (`:140-183`).

**Motion**, two modes (`:428-603`):
- **Rail mode:** follows the track's checkpoint nodes (restart points), advancing **112 lu per frame** along the polyline. It takes the branch the target took (`NewPathNode`, `:75-109`).
- **Homing mode:** used once the target is within 3 nodes. Steers at the target: yaw rate 0x400 when near, down to 0x100 farther out (64 while in owner safety); speed 112 lu/frame; vertical speed ±4 per frame toward the target's height, capped at 96.

**Hit** (any driver except the owner inside 192 lu XZ):
- the driver is **blasted** (`:701-772`);
- **normal:** the orb dies if it hit its target or the leader;
- **juiced:** it marks the victim and everyone behind them as hit, retargets the next driver ahead, and dies on the leader.

**Turnaround and death.** On a weapon-react trigger, a hitbox or lost ground, it reverses along the path (`RB_Warpball_TurnAround`, `:296-370`). It dies after **120 turnaround frames** or with no target. Death SFX **0x4F**, then a 6-frame fade (`:12-73`).

### 3.12 Invisibility (ID 12, battle only)

- Duration 7680 / 11520 ms (`VehPickupItem.c:1497-1528`).
- Makes the kart transparent; others can't see it (`RB_Player.c:200-226`).
- Missiles can't lock on to it and lose it if already locked (`VehPickupItem.c:425`, `RB_MovingExplosive.c:69-72`).
- SFX **0x61** on, **0x62** reappear.

---

## 4. Hit reactions on the victim

Entry point: `RB_Hazard_HurtDriver(victim, damageType, attacker, reason)` (`231/RB_Hazard.c:4-23`).
- Bots go to `BOTS_ChangeState`.
- Humans go to `VehPickState_NewState` (`Vehicle/VehPickState.c:48-381`).

**Gatekeeping** (`VehPickState.c:65-114`):
1. Mask-grabbed: ignored.
2. Mask active or `invincibleTimer != 0`: ignored; the victim laughs (voice 2) and the function returns 0.
3. Shield up: the shield pops, `invincibleTimer = 672 ms`, returns 0.
4. Otherwise apply the damage type below.

**Common effects** of any applied damage (`:299-306`):
- `reserves = 0` (the boost is cancelled);
- `turbo_outsideTimer = 0`;
- 8-frame rumble;
- the new state installs on the **next** frame through `funcPtrs[INIT]`.

| Type | Caused by | Lock-out (`NoInputTimer`) | State and motion | Fruit | Cite |
|---|---|---|---|---|---|
| 1 **Spin-out** | beaker, clock, spiders, hazards | **960 ms = 30 frames** (if already spinning, the timer is just reset; no re-init) | `KS_SPINNING`. `baseSpeed = 0`, brake held → friction `BrakeFriction/2` = 130 per frame on both axes. Visual yaw spins at ±300/frame. **SpinFirst** lasts while the timer runs **and** \|speed\| > 767. Then **SpinLast** recentres the kart (until \|turnAngle\| < 16), then **SpinStop** plays the animation back to centre and returns to Driving. Total time depends on speed. | −1 | `VehPickState.c:238-246`; `VehPhysProc.c:2766-3150`; `VehPhysForce.c:455-468` |
| 2 **Blasted** (flip in the air) | bomb, missile, TNT, Nitro, warp orb, shield bomb, mask/shield ram | **2400 ms = 75 frames** | `KS_BLASTED` (`VehStuckProc_Tumble_*`). `baseSpeed = 0`, brake friction ×3/4 = 195 per frame, friction applies even airborne. Every grounded frame re-launches the kart with `jump_InitialVelY = 6000 + 2*NoInputTimer` (about 10800 at the start, versus a normal hop of 4596), so it bounces. Baked tumble rotation from `bakedGteMath[BLASTED]`, forward or backward at random (`rng & 4`). Ends straight into Driving when the timer hits 0. **A new blast is ignored while blasted.** | −3 | `VehPickState.c:151-184`; `VehStuckProc.c:1349-1504`; `VehPhysGeneral.c:1348-1364`; `VehPhysForce.c:461-464` |
| 3 **Squished** | turbo ram, Baron plane, snowballs, minecarts | 240 ms (7.5 frames) | `squishTimer = 3840 ms` (120 frames): kart drawn flat (`scale.y = 0`, `VehPhysForce.c:1737-1740`), top speed −4096. Also starts a spin-out. SFX **0x5A**, voice 4. | −1 (spin) | `VehPickState.c:227-236`, `VehPhysGeneral.c:2093-2096` |
| 4 Burn | fire hazards | 1920 ms (60 frames) | `burnTimer = 3840 ms`, top speed −4096, plus a spin-out. SFX **0x69**. | −1 | `VehPickState.c:216-225` |
| 5 Mask-grab | plants | 3360 ms (105 frames) | `VehStuckProc_PlantEaten_*` | −2 | `VehPickState.c:186-206` |

**Invulnerability after a hit:**
- In races, there is **no** general post-hit i-frame timer. The only protection is "no re-blast while blasted" and "re-spin only resets the 30-frame timer".
- **Battle mode:** leaving the blasted state gives `invincibleTimer = 2880 ms` (**90 frames**) (`VehPhysProc.c:1706-1709`). The kart flickers while `invincibleTimer > 672` (`RB_Player.c:228-257`).
- Shield pop: 672 ms (21 frames).

**Other victim feedback** (visual only):
- the HUD icon tints red (or green for Nitro or green beaker) for 30 frames (`damageColorTimer`);
- a white screen flash for human victims of explosions and potions (`RB_Burst.c:111-118`, `RB_GenericMine.c:183-194`).

---

## 5. Where the code lives (for the port)

| Area | Functions | File |
|---|---|---|
| Crate break and respawn | `RB_CrateWeapon_ThCollide/LInC`, `RB_CrateFruit_*`, `RB_CrateAny_ThTick_Grow` | `231/RB_Crate.c` |
| Fruit | `RB_Fruit_ThCollide/LInC`, `RB_Player_ModifyWumpa`, HUD credit queue | `231/RB_Fruit.c`, `231/RB_Player.c`, `UI/UI_RenderFrame.c:212-257` |
| Roulette tick, fire input, ammo | `VehPhysProc_Driving_PhysLinear` (item roll `:576-607`, item timers `:612-646`, Circle `:801-893`) | `Vehicle/VehPhysProc.c` |
| Item choice | `VehPhysGeneral_SetHeldItem`; tables `RNG_itemSet*` | `Vehicle/VehPhysGeneral.c:1651-1962`; `zGlobal_DATA.c:8108-8174` |
| Firing | `VehPickupItem_ShootOnCirclePress`, `VehPickupItem_ShootNow`, `_MissileGetTargetDriver`, `_PotionThrow`, `_MaskUseWeapon` | `Vehicle/VehPickupItem.c` |
| Bomb, missile, shield bomb | `RB_MovingExplosive_ThTick/_Explode` | `231/RB_MovingExplosive.c` |
| Explosion damage | `RB_Burst_Init` (bomb/missile), `RB_Blowup_Init` (TNT/Nitro), `RB_Explosion_*` (visual only) | `231/RB_Burst.c`, `231/RB_Blowup.c`, `231/RB_Explosion.c` |
| Mines | `RB_GenericMine_ThTick/_ThDestroy`, `RB_MinePool_*`, `RB_Potion_ThTick_InAir`, `RB_TNT_ThTick_*` | `231/RB_GenericMine.c`, `RB_MinePool.c`, `RB_Potion.c`, `RB_TNT.c` |
| Contact tests | `RB_Hazard_CollideWithDrivers/Bucket`, `RB_Hazard_ThCollide_*`, `RB_Hazard_InterpolateValue`, `PROC_CollideHitboxWithBucket` | `231/RB_Hazard.c`, `PROC.c:480-560` |
| Shield, mask, rain cloud | `RB_ShieldDark_ThTick_Grow/_Pop`, `RB_MaskWeapon_ThTick/_FadeAway`, `RB_RainCloud_*` | `231/RB_MaskShieldCloud.c` |
| Warp orb | `RB_Warpball_*` | `231/RB_Warpball.c` |
| Damage → state | `RB_Hazard_HurtDriver`, `VehPickState_NewState` | `231/RB_Hazard.c`, `Vehicle/VehPickState.c` |
| Hit states | `VehPhysProc_Spin{First,Last,Stop}_*`, `VehStuckProc_Tumble_*`, `VehPhysProc_Driving_Init` | `Vehicle/VehPhysProc.c`, `Vehicle/VehStuckProc.c` |
| Kart-on-kart weapon effects | `VehPhysCrash_Attack` | `Vehicle/VehPhysCrash.c:199-249` |
| Bot weapon AI | `PickupBots_Update*` | `PickupBots.c` |

---

## 6. Sound effect IDs (`OtherFX_Play` / `PlaySound3D` arguments)

| ID | Event | Cite |
|---|---|---|
| 0x3C | any crate breaks | `RB_Crate.c:150` |
| 0x5D | roulette loop (start / stop) | `RB_Crate.c:202`, `UI_RenderFrame.c:1056` |
| 0x5E / 0x41 | roulette result: normal / juiced | `VehPhysProc.c:21-22` |
| 0x41 | reached 10 Wumpa | `RB_Player.c:194` |
| 0x43 / 0x42 | fruit touched (3D) / fruit credited to the counter | `RB_Fruit.c:59`, `UI_RenderFrame.c:234` |
| 0x47 / 0x48 / 0x49 | bomb launch / roll loop / explode | `VehPickupItem.c:151`, `RB_MovingExplosive.c:50`, `379` |
| 0x4A / 0x4B / 0x4C | missile launch / loop / explode | `VehPickupItem.c:152`, `RB_MovingExplosive.c:43`, `392` |
| 0x52 | TNT, Nitro or beaker drop | `VehPickupItem.c:153` |
| 0x50 / 0x51 / 0x3E / 0x3D | TNT bounce onto head / lands on head / fuse honk / TNT explosion | `RB_GenericMine.c:284`, `RB_TNT.c:289`, `208`, `32` |
| 0x3F | glass shatter (Nitro, beaker) | `RB_Hazard.c:187`, `198` |
| 0x57 / 0x58 / 0x59 | shield up / shield pop or fire / shield-bomb roll loop | `VehPickupItem.c:154`, `RB_MaskShieldCloud.c:261`, `RB_MovingExplosive.c:56` |
| 0x4F | warp orb death; also the bubble pop on a shield ram | `RB_Warpball.c:66`, `VehPhysCrash.c:21` |
| 0x4D / 0x4E | warp orb launch / loop | `VehPickupItem.c:156`, `RB_Warpball.c:605` |
| 0x44 | clock | `VehPickupItem.c:155` |
| 0x53 / 0x54 | Aku Aku / Uka Uka | `VehPickupItem.c:167-168` |
| 0x61 / 0x62 | invisibility on / off | `VehPickupItem.c:157`, `VehPhysProc.c:20` |
| 0x5A / 0x69 | squish / burn | `VehPickState.c:33-34` |

**Voice lines** (`Voiceline_RequestPlay` id):

| Id | Line | Id | Line |
|---|---|---|---|
| 1 | took damage | 11 | missile |
| 2 | victim laugh (blocked hit) | 12 | warp orb |
| 3 | spin | 13 | shield fire |
| 4 | squished | 14 | clock |
| 10 | bomb | 15 | mine drop |

(`VehPickupItem.c:158-163`, `VehPickState.c:19-22`)

---

## 7. Port notes

- Run the weapon threads after the driver stages, as CTR does (`MainFrame.c:294-330`, buckets in order). Fire requests are latched in PHYS_LINEAR and consumed at the start of the next frame.
- Weapon motion is in whole lu per frame. Keep it separate from the kart's 1/256-lu integer state, or scale by 256.
- Weapon "walls" are data (weapon-react trigger quads). On WoW meshes, use a sweep against steep faces (and WMO walls) and treat a hit as that trigger.
- The track-following warp orb and the AI "driver ahead" fallbacks need a checkpoint polyline with distance-to-finish. Reuse the race path, if the port has one.
- RNG: one global 16-bit LCG feeds item rolls, Wumpa crate amounts, TNT jump counts and rain-cloud effects (`MixRNG_Scramble`). The roulette icon, crate break-up rotation and similar visuals use libc `rand()`.

## 8. Not covered, or uncertain

- **Crate and fruit hitbox sizes** are level data (BSP hitbox boxes), not code constants. Use the model's bounds.
- **Single Wumpa fruit never respawn** in the code that was read: scale stays 0 and nothing found resets it. This was not verified in play.
- **Bots don't use crates** for items. `PickupBots.c:66-255` spawns their shots when near a human: for a bot just ahead of a human, 1/200 per frame for TNT/beaker, bomb or missile; for a bot behind, 2/800 missile and 2/800 bomb; then a 240-495 frame cooldown. Their damage goes through `BOTS_ChangeState` (`BOTS.c`), which was not detailed here.
- **Backward bomb with a lock-on target** flies away from the target at full speed after its first tick (`RB_MovingExplosive.c:129-143`). This is literal code behaviour; whether it is noticeable in play was not checked.
- **Warp orb path flags** (`TRACKER_FLAG_WARPBALL_*`) are summarised, not traced exhaustively. Follow `RB_Warpball.c` line by line for an exact port.
- **Blasted bounce height** comes out of the physics spec's jump code (§6.3) driven by `jump_InitialVelY`. The real arc and duration were not simulated here.
- **Clock "waddle"** steering wobble: only the rumble was traced.
- **The adventure "boss weapon" metadata** (`GAME_BOSS_WEAPON_METADATA`) is not covered.
