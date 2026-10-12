# CTR weapons: where the assets are and how they could go over the wire

Research notes for adding CTR weapon crates, Wumpa fruit and weapons to World of Kartcraft.
Part A covers the disc assets. Part B covers the client/server protocol as it is today and
proposes a `W` verb.

Disc facts below were measured on the NTSC-U image (SCUS-94426) with a scratch tool built on
`ctr_assets` (`mpk::models`, `mpk::icons`, `Model::parse_at`, `Vram`, `audio::Howl`), not
taken from documentation alone. Code references are CTR-native (`ctr-native-ref/`) unless
another source is named.

---

## A. Assets

### A1. Models

Weapon, crate-effect and mask models are in **`packs\shared.mpk` (bigfile 259)**, the pack
`effects.rs` already reads for the turbo flames. The `1P_arcade\<racer>.mpk` packs (260+) carry
the same models again, so nothing else needs reading. In the table below, `id` is the model's
`gGT->modelPtr` id, i.e. the `MODEL_ID` enum in `include/namespace_Instance.h`.

**Every texture of these models resolves in `shared.vrm` (258) alone.** Checked texel by texel
for each tex layout's UVs and CLUT. The existing `effects::mesh_frame` path therefore works
unchanged.

| thing | model name in shared.mpk | id | meshes / animation |
|---|---|---|---|
| Bowling bomb | `bomb` | 0x3B `DYNAMIC_BOMB` | `bomb_hi` 60 tris, `bomb_lo`; static |
| Missile (homing) | `rocket` | 0x29 `DYNAMIC_ROCKET` | `rocket_hi` 32 tris, `rocket_lo`; anim `thrust` ×10 |
| TNT crate (dropped) | `crate_tnt` | 0x27 `STATIC_CRATE_TNT` | `_hi` 48 / `_low` 12; static |
| Nitro crate (juiced TNT) | `crate_nitro` | 0x06 (`PU_EXPLOSIVE_CRATE`) | `_hi` 48 / `_low` 12; static |
| Crate-break / explosion debris | `crate_explosion` | 0x26 `STATIC_CRATE_EXPLOSION` | 22 tris; anim `break` ×12 |
| Beaker / potion | `beakerred` (normal), `beakergreen` (juiced) | 0x46 / 0x47 | `beaker_hi` 40 tris, anim `bubble` ×15; `beaker_lo` |
| Potion hit cloud over a driver | `cloud` | 0x42 `STATIC_CLOUD` | anim `billow` ×23 |
| Potion/bomb shockwave | `shockwavered`, `shockwavegreen` | 0x44 / 0x45 | anim `wave` ×17 |
| Shield bubble | `shield` (normal), `shield_green` (juiced), `shielddark`, `highlight` | 0x56, 0x5E, 0x5A, 0x5D | `shield_hi` 48 tris, static; `highlight_hi` 28 tris |
| Aku Aku mask (in race) | `akumouth` (+ `akubeam`) | 0x39 `STATIC_AKUAKU`, 0x3E | `akumouth` 49 tris, anim `speak` ×7; `akubeam_hi` anim `turn` ×24 |
| Uka Uka (villains) | `ukamouth`, `ukabeam` | 0x3A, 0x40 | same layout as Aku |
| Warp orb | `warpball` (+ `warpedburst` on hit) | 0x36 `DYNAMIC_WARPBALL`, 0x2B | `warpball_hi` 56 tris, anim `warp` ×16; `burst_hi` anim `burst` ×30 |
| HUD Wumpa | `fruitdisp` | 0x37 | already exported in hud.json |
| Clock | **no model** | — | the clock is a screen effect plus a sound (`VehPickupItem.c` ~864) |

**Weapon crate ("?"), fruit crate and Wumpa fruit are not in shared.mpk.** Every track's level
file has its own copies (`levels\tracks\<trk>\1P\data.lev`; Crash Cove = bigfile 113, Dingo
Canyon = 17). Level header (CTR-tools `SceneHeader.cs`): `numModels @0x14`,
`ptrModelsPtr @0x18`, `numInstances @0x0C`, `ptrInstances @0x10`. The pointer list is inside
the `unwrap_patched` data.

| thing | lev model | id | meshes |
|---|---|---|---|
| Weapon crate "?" | `crate_question` | 0x08 `PU_RANDOM_CRATE` | 60 tris hi, 12 low; static |
| Fruit crate | `crate_fruit` | 0x07 `PU_FRUIT_CRATE` | 60 / 12; static |
| Wumpa fruit (pickup) | `fruit` | 0x02 `PU_WUMPA_FRUIT` | `fruit_hi` 52 / `med` 34 / `low` 18 / `box` 4, all **untextured Gouraud**, anim `spin` ×32 |

The crate textures live in the level's own VRM (`data.vrm`, e.g. 112 for Crash Cove 1P). With
shared.vrm + level vrm loaded, every crate texel resolves. With shared.vrm alone the crate pages
are unwritten. So extracting the crates means loading one track's `.vrm` (any track works; they
are the same crates). Crash Cove 1P has 16 `crate_question#`, 3 `crate_fruit#` and 10 `fruit#`
instances. Instance records are 0x40 bytes with a 16-byte name (position is at a fixed offset
per CTR-tools `Instance`). That is enough if CTR's own crate layout is ever wanted as a
reference for spacing.

### A2. HUD icons (item slot, roulette, Wumpa counter)

`hud.json` **already has them**:

- Item icons `item_turbo, bomb, missile, tnt, potion, shield, aku_mask, clock, warpball,
  bomb_x3, missile_x3` plus the juiced variants `item_tnt_juiced, potion_juiced, shield_juiced`,
  `item_uka_mask` and `item_juiced_shine`.
  - Source: `ptrIcons[heldItemID + 5]` in shared.mpk.
  - On-disc icon names: 5 `turbonew`, 6 `bomb`, 7 `bomb-tracker` (missile), 8 `nitro` (TNT),
    9 `poison`, 0xB `shield`, 0xC `doctor` (Aku), 0xD `clock`, 0xE `missile` (the warp orb),
    0xF `bomb3`, 0x10 `bomb-tracker3`, 0x14 `nitroj`, 0x15 `poisonj`, 0x17 `shieldj`,
    0x31 `poweredup`, 0x32 `ukauka`.
- **Roulette:** there is no separate roulette art. `hud_draw` already handles `item = (0x10, _)`
  by showing a random racing item each frame (`hud_draw.rs` ~435). `HudState.item` exists but
  `kartcraft::hud::HudExtras` has no item field yet. Adding one is all the client HUD needs.
- **Wumpa counter:** the `fruitdisp` mesh (spinning) plus the "x" and digits, already drawn from
  `HudState.wumpa`, and `HudExtras.wumpa` is wired.
- Not exported, and possibly wanted:
  - icon 0x2D `target`: the missile lock-on square drawn on a targeted player
    (`UI_RaceHud.c`, beep sfx 0x56);
  - icon group 11 `wumpaposter`.
  - Spring (id 5), invisibility (12) and super engine (13) have **no icons in shared.mpk**
    (indices 0x0A, 0x11, 0x12 are absent). They are battle or unused items, so leave them out.

### A3. Sounds (`SOUNDS\KART.HWL`, OtherFX ids)

**None of these are extracted yet.** `audio.rs::SOUNDS` holds 11 kart sounds. Every id below
resolves to a sample in the HOWL banks that `Howl::parse` already loads (checked), so adding
`SoundSpec` rows is enough.

| event | OtherFX id | source | rate / length / loop / instr. volume |
|---|---|---|---|
| Roulette spinning | **0x5D** | `RB_Crate.c` ~202, `OtherFX_Play(0x5d,0)` when a "?" crate gives a roll | 12371 Hz, 0.17 s, **loop**, 153 |
| Roulette stops (item chosen) | **0x5E** | `VehPhysProc.c` ~594 `ITEM_ROLL_NORMAL_FX` | 6944 Hz, 0.64 s, 204 |
| Roulette stops with ≥10 wumpa ("ka-ching") | **0x41** | `ITEM_ROLL_JUICED_FX` | 11025 Hz, 0.92 s, 229 |
| Crate break (any crate) | **0x3C** | `RB_Crate.c` `PlaySound3D(0x3c)` | 9991 Hz, 0.68 s, 178 |
| Wumpa pickup (3D, at the fruit) | **0x43** | `RB_Fruit.c` ~60 | 15999 Hz, 0.22 s, 229 |
| Wumpa counted into HUD | **0x42** | `UI_RenderFrame.c` ~235 | 11025 Hz, 0.21 s, 102 |
| Bomb launch | **0x47** | `VehPickupItem.c` `SOUND_BOMB_LAUNCH` | 9991 Hz, 0.61 s |
| Bomb rolling | **0x48** | `RB_MovingExplosive.c` ~50 | 8742 Hz, loop |
| Bomb explosion | **0x49** | `RB_MovingExplosive_Explode` | 8000 Hz, 1.34 s |
| Missile launch | **0x4A** | `SOUND_MISSILE_LAUNCH` | 11025 Hz, 1.05 s, 204 |
| Missile flight | **0x4B** | `RB_MovingExplosive.c` ~43 | 12995 Hz, loop |
| Missile explosion | **0x4C** | `RB_MovingExplosive_Explode` | 8000 Hz, 1.06 s |
| Missile lock-on beep | 0x56 | `UI_RaceHud.c` `UI_TRACKER_BEEP_SFX` | 8000 Hz, 0.15 s, 102 |
| Mine (TNT/potion) drop | **0x52** | `SOUND_MINE_DROP` | 12995 Hz, 0.46 s, 102 |
| TNT lands on a head | 0x50 | `RB_GenericMine.c` ~284 | 11025 Hz, 0.65 s |
| TNT jump/hit cue | 0x51 | `RB_TNT.c` ~289 | 14998 Hz, 0.36 s |
| TNT countdown "honk" | 0x3E | `RB_TNT.c` ~208 | 8990 Hz, 0.15 s |
| TNT / Nitro explosion | **0x3D** | `RB_TNT.c`, `RB_GenericMine.c` | 14998 Hz, 2.17 s |
| Nitro / beaker glass shatter | 0x3F | `RB_GenericMine.c` ~397 | 11994 Hz, 1.14 s |
| Shield up | **0x57** | `SOUND_SHIELD` | 11025 Hz, 0.58 s, 204 |
| Shield pop | 0x58 | `RB_MaskShieldCloud.c` | 11025 Hz, 0.58 s |
| Shield thrown (rolling) | 0x59 | `RB_MovingExplosive.c` ~56 | loop |
| Aku Aku | **0x53** (`STATIC_AKUAKU + 0x1A`) | `VehPickupItem.c` ~258 | 13997 Hz, 1.38 s |
| Uka Uka | 0x54 | same | 13997 Hz, 0.64 s |
| Clock | **0x44** | `SOUND_CLOCK` | 11025 Hz, 1.55 s |
| Warp orb launch / flight / hit | 0x4D / 0x4E (loop) / 0x4F | `VehPickupItem.c`, `RB_Warpball.c` | 11025 / 12995 / 11025 Hz |

Spin-out and hit **voice lines** are XA (not HOWL) and are not covered here.

### A4. Gameplay constants worth carrying over (decomp)

| what | value | source |
|---|---|---|
| Roulette length | `itemRollTimer = 90` frames (3 s at 30 fps); a fire press after 70 frames stops it early (`ITEM_ROLL_FAST_STOP_FRAMES`) | `RB_Crate.c` ~198, `VehPhysProc.c` ~13/898 |
| Crate respawn | `cooldown = 0x1E` (30 frames = 1 s), then it grows back at `+0x100`/frame (16 frames ≈ 0.53 s); total ≈ 1.5 s | `RB_Crate.c` ~48–75, 125 |
| Cooldown pause | the cooldown pauses while a driver or a mine sits in the crate's spot (`boolPauseCooldown`) | same |
| Fruit crate payout | `rand % 4 + 5` = 5–8 wumpa | `RB_Crate.c` ~316 |
| Loose fruit | 1 each and **does not respawn** during the race (scale stays 0) | `RB_Fruit.c` |
| Wumpa cap | `DRIVER_WUMPA_JUICED_COUNT = 10`: juiced at 10, capped at 10 | `namespace_Vehicle.h:668` |
| Item choice | rank-weighted random | `VehPhysGeneral_SetHeldItem` (`VehPhysGeneral.c:1681`) |

### A5. Extending `ctr-extract`: concrete next steps

1. **`src/weapons.rs`** with `pub fn build(bf: &mut BigFile) -> Result<WeaponsFile>`, modelled on
   `effects::build`:
   - Load `shared.vrm` and `shared.mpk`. Take models by **id** with `mpk::models` (ids above;
     look up by id, as effects.rs does, because `crate_nitro`'s id 0x06 collides with nothing
     else in the pack).
   - Reuse `effects::Textures` and `effects::mesh_frame`. Make them `pub(crate)` and add a
     variant that takes a `&Frame`, because `mesh_frame` uses only `mesh.frame` today.
   - Export every anim frame: `mesh.anims[i].frames[j]`, then `mesh.triangles(frame)`. That
     covers rocket `thrust`, beaker `bubble`, akumouth `speak`, warpball `warp`, cloud
     `billow`, shockwave `wave`, crate_explosion `break` and warpedburst `burst`.
   - Use the `_hi` mesh (`meshes[0]`) only.
2. **Level crates:** open one track (Crash Cove 1P: `.vrm` 112 and `.lev` 113; add these to
   `big::index` as, e.g., `TRACK_CRASH_COVE_1P_VRM/LEV`). Then:
   - load the vrm on top of shared.vrm;
   - follow `ptrModelsPtr @0x18` to `crate_question`, `crate_fruit` and `fruit`, and parse them
     with `Model::parse_at`;
   - export the fruit's `spin` ×32 frames, untextured, whose vertex colour is the colour
     (divide by 255, as `mesh_frame` already does for untextured).
3. **`weapons.json`** schema in the spirit of effects.json: `textures[]`, then
   `models{ name: { source, frames:[{surfaces}], frame_rate_fps: 30, animation } }` for
   `crate_question, crate_fruit, fruit, bomb, rocket, crate_tnt, crate_nitro, crate_explosion,
   beaker_red, beaker_green, cloud, shockwave_red, shockwave_green, shield, shield_green,
   shield_dark, highlight, aku, uka, akubeam, ukabeam, warpball, warpedburst`. Add a
   `constants` block with the A4 numbers.
   - The units are effects.rs units: world units / 51.2 = m. Check the instance scale per model
     against the decomp's `scale` in each weapon's birth before trusting sizes.
     For example, crates spawn at 0x1000.
4. **Sounds:** append `SoundSpec` rows to `audio::SOUNDS`, which then needs more than 11
   entries:
   - `roulette.wav` (0x5D, loop), `roulette_stop.wav` (0x5E), `roulette_juiced.wav` (0x41);
   - `crate_break.wav` (0x3C), `wumpa.wav` (0x43), `wumpa_count.wav` (0x42);
   - `bomb_launch.wav` (0x47), `bomb_roll.wav` (0x48 loop), `bomb_explode.wav` (0x49);
   - `missile_launch.wav` (0x4A), `missile_fly.wav` (0x4B loop), `missile_explode.wav` (0x4C),
     `missile_lock.wav` (0x56);
   - `mine_drop.wav` (0x52), `tnt_land.wav` (0x50), `tnt_honk.wav` (0x3E), `tnt_explode.wav`
     (0x3D), `glass.wav` (0x3F);
   - `shield.wav` (0x57), `shield_pop.wav` (0x58), `aku.wav` (0x53), `uka.wav` (0x54),
     `clock.wav` (0x44);
   - `warp_launch.wav` (0x4D), `warp_fly.wav` (0x4E loop), `warp_hit.wav` (0x4F).

   Then extend the `tests/disc.rs` expectations and the `docs/ctr-assets.md` table.
5. **HUD:** add `item: Option<(u8,u8)>` to `kartcraft::hud::HudExtras` and pass it through to
   `HudState.item`. Optionally export icon 0x2D `target` in `hud.rs`.
6. **CLI:** `--weapons` (and part of `--all`) writes `<out>/weapons.json`. Add a
   `render_weapons` example, a copy of `render_effects`, for visual verification.

---

## B. Network: how karts and races travel today

### B1. Transport

- **Client → server.** `ClientCommand::AddonMessage { distribution: Party, text }`
  (`kart/profession.rs::send`) becomes `CMSG_MESSAGECHAT`, chat type PARTY (1), language
  `LANG_ADDON` (0xFFFFFFFF), text `"KARTCRAFT\t<verb>\t..."` (`benilla-protocol` `send_addon_message`).
  1.12 has no addon opcode.
- **Server hook.** `kartcraft.patch` inserts this into vmangos
  `WorldSession::HandleChatMessageOpcode`:

  ```cpp
  if (packet.lang == LANG_ADDON && _player && Kartcraft::HandleAddonMessage(_player, packet.message)) return;
  ```

  - It runs **before** the language, chat-type and group checks. A group-less "party" line
    therefore still reaches us, and every `KARTCRAFT\t...` line is swallowed (never relayed as
    real chat).
  - `HandleAddonMessage` dispatches on the first two bytes: `L\t`, `S\t`, `R\t`.
- **Server → client.** `ChatHandler::BuildChatPacket(CHAT_MSG_PARTY, text, LANG_ADDON, sender guid, name)`.
  - The relay is `SendMessageToSetInRange` (`RelayState`, race offers); the direct send is
    `SendPacket` (`SendLine`, `SendRace`).
  - Clients see it as an addon chat line. `SessionEvent::Chat` with `m.is_addon()`, then
    `strip_prefix("KARTCRAFT\tS\t")` in `remote::on_chat` or `"KARTCRAFT\tR\t"` in
    `race::on_chat`, both registered as net handlers in `kart/mod.rs:405-406`.
  - For relays, the **sender guid is the driver's** (the server builds the packet with
    `player->GetObjectGuid()`). That is how `remote.rs` keys states by `m.sender_guid`.
- **Size limits.**
  - The 1.12 client's `SendChatMessage`/`SendAddonMessage` truncates at **255 characters**
    (`0x49f604`).
  - benilla does not truncate. Per `benilla-ui/src/script/chat_send.rs:15`, vmangos refuses a
    longer line itself (`Chat.cpp:2177`), so an over-long line is **dropped**, not cut.
  - Treat **255 bytes for the whole `KARTCRAFT\t...` string** as the hard ceiling and design
    for ≤ 200.
  - Kartcraft adds its own limit: `MAX_STATE_LENGTH = 120` for `S` (`remote.rs` test asserts
    < 120).
  - `R\tC` (checkpoint list) is the longest line today: 6–8 points of `x:y;` ≈ 100 bytes.
- **Flood.** `LANG_ADDON` skips vmangos's chat flood and sanitize checks, so Kartcraft's own
  per-player timers are the only rate limits.

### B2. Verbs today

| verb | direction | format | server handling |
|---|---|---|---|
| `L` | C→S | `KARTCRAFT\tL\t<points>` (a banked turbo chain) | `LandLine`: needs Summon Kart aura; ≥ `LINE_INTERVAL` 1000 ms per player; points clamped to 1e5; raises the Karting skill +0..5 and gives XP. Not relayed. |
| `S` | C→S→others | `KARTCRAFT\tS\t<steer×100>,<drift×1000>,<fire>,<engine distortion>,<engine volume>,<flags>,<racer>`; flags bit0 grounded, 1 drifting, 2 boost window, 3 failed boost | `RelayState`: needs aura; ≤ 120 bytes; ≥ `STATE_INTERVAL` 50 ms per player (else dropped); the **same line** is relayed to every player within `STATE_RANGE` 150 yd, **not to the sender**, with no group needed. Client sends every `STATE_PERIOD` 0.1 s (`kart/mod.rs:24,334`). The receiver keeps it for `STATE_TTL` 0.5 s. |
| `R` | both | see B3 | `HandleRace` |

Position itself does **not** travel in `S`. Remote karts are placed from ordinary
`MSG_MOVE_*` packets (`net::motion`, `RemoteMotion`) and interpolated 0.15 s behind
(`remote.rs` `DELAY`). `S` only carries look, sound and the racer skin.

Kart presence is the **Summon Kart aura** (spell 60001, a mount aura using a mechanostrider's
display): `drives_kart()` checks `unit_auras()`.

### B3. Races (`R`)

Client → server:

- `R\tH\t<courseKey>[\t1]`: host. The player must be within `START_RADIUS` 60 yd of point 0 on the
  course's map. A trailing `1` hosts with ground types on (`GT`); without it they are off. A race
  already waiting on the course takes the player in and keeps its own ground types.
- `R\tJ\t<id>`: join. Only before the start, at most 8 racers, near the start, no duplicates.
- `R\tS\t<id>`: start. Host only.
- `R\tK\t<id>\t<index>`: checkpoint passed.
- `R\tQ\t<id>`: leave.
- `R\tT\t<courseKey>`: a time trial, from the Race Marshal's menu. The player leaves the queue,
  any ready check (as a no) and every race, and races alone: the server answers with the roster
  (`L`), then `TT`, puts them on grid slot 0 and sends the course, countdown and records as a
  start does. Nobody can join it (`J`, `LJ` and `H` pass it by), the Race Finder's lobby never
  lists it, and it never takes a pop.

Server → client:

- `R\tO\t<id>\t<key>\t<name>\t<host>`: offer, to players within 120 yd **including** the host.
- `R\tL\t<id>\t<roster csv>`: roster, to the racers.
- `R\tGT\t<id>\t<0|1>`: the race's ground types, at the start just before `C`, to every racer.
  `1`: grass, dirt, snow, mud and water slow every racer as CTR's grounds do; `0`: asphalt
  everywhere. From the countdown until a racer finishes or leaves, the client drives by this and
  not by its own `KartTerrain` (`kart::race::Races::ground_types`), so every racer drives the
  same course; outside a race the setting applies. A hosted race takes the host's choice (the
  Race Marshal's "Host" page, default off); a Race Finder race `FINDER_GROUND_TYPES` (off); a
  time trial is always off, so its leaderboard stays one board of like runs. A client that hears
  no `GT` races off. Hosted races with ground types on share the races' board: they can only be
  slower.
- `R\tG\t<id>\t<countdown ms 4000>\t<roster>`: countdown.
- `R\tP\t<id>\t<name>\t<lap>\t<checkpoint>`: progress.
- `R\tF\t<id>\t<name>\t<place>\t<ms>[\t<r|p>]`: finish; `r` a new course record, `p` a new
  personal best, on the race's board.
- `R\tTT\t<id>`: race `<id>` is a time trial, sent right after its first roster. No crate layout
  (`W\tL`) follows and every `W` line from its racer is refused; the client draws no crates or
  items, and records and replays its ghost only in a time trial.
- `R\tRec\t<id>\t<key>\t<record ms>\t<holder>\t<own best ms>`: at the start and after a new
  best; a time trial's come from its own board (`kartcraft_tt_records`), a race's from the races'
  (`kartcraft_records`).

How the server checks and stores race state:

- **`K` is server-validated.** The index must be the next one in order (`(cp+1)%n` for laps,
  `cp+1` for sprints). The server checks the player's **real server position** (2D) within
  `CHECKPOINT_RADIUS` 45 yd of that point; the client uses 38 yd to leave slack. The race must
  have passed GO. A lap counts when index 0 comes round again.
- **State.** `static std::unordered_map<uint32, Race> s_races`. Each `Race` holds the course
  pointer, host guid, created and start times, a finisher count and `vector<Racer{guid, name,
  lap, checkpoint, finished}>`.
  - Races older than `RACE_LIFETIME` 20 min are purged lazily, on every `R` line.
  - A race is erased when everyone has finished or left.
  - Rewards: XP and Karting skill by place.
- **Course data.**
  - One source: `courses/courses.json` (`docs/courses.md`). `pack.py` makes it into
    `KartcraftCourses.h`, the server's `COURSES`; the client reads it at compile time
    (`kart/race.rs` `COURSES`) for keys, names, starts and gate widths.
  - The client learns the full course only from `R\tC` at the start.
- **Placing.** Done client-side from `R\tP` (`race.rs` `standing`).

Server sources live in `setup/vmangos/src/Kartcraft.{h,cpp}`. **After editing them, run
`python3 setup/vmangos/pack.py`**, which regenerates the new-file hunks of `kartcraft.patch`.
The patch is what the server build applies. Today the two are in sync.

### B4. Adding `W` (weapons): recommended split

Principle: **the server owns scarce, shared, rewardable state** (crates, what you hold, wumpa).
**The firer's client simulates projectiles. The victim's client decides it was hit.** This is
the usual latency-hiding split. Positions already arrive late by 0.15 s and up to 150 yd away,
so server-side projectile simulation without WoW-side physics would be guesswork.

**Server-authoritative**

- **Crate layout.** Add `crates` (`x, y, z, type` where `type` is `?` or fruit) and
  `fruit` points to `Course`, next to `points`, so one table stays the source of truth.
  - Global per course, not per race: the world is shared, and anyone near a course should see
    the same crates.
  - Lazily create `struct CrateState { uint32 respawnAt; }` arrays keyed by course.
- **Claims** (`W\tC\t<course>\t<crate>`). Accept when the crate is up and the player's server
  position is within ~8 yd. The crate radius is small, so allow for the 150 ms lag at kart
  speed, a few yd. Then:
  - mark the crate down until `now + CRATE_RESPAWN`, about 1.5 s per A4 (pausing while occupied
    is optional);
  - for `?`, roll the item server-side and reply `W\tI\t<item>\t<count>` to the claimer only.
    The server knows race places from `R` progress and can weight the roll like
    `VehPhysGeneral_SetHeldItem`. Remember `held[guid]`.
  - for fruit crates, add 5–8 wumpa and reply `W\tU\t<wumpa total>`, cap 10;
  - broadcast `W\tT\t<course>\t<crate>\t<respawn ms>` to players within 150 yd (including the
    taker, like `R\tO`), so everyone hides the crate and plays the break sound.
  - The client waits for `I`, which takes one RTT. That is hidden behind the 3 s roulette,
    which can start spinning optimistically on touch.
- **Loose fruit.** Same pattern: `W\tC` with a fruit index, no respawn for the race. Outside a
  race, use a long respawn.
- **Fire** (`W\tF\t<seq>\t<item>\t<x>,<y>,<z>,<yaw>[\t<targetGuidLow>]`). Accept only if
  `held[guid]` holds that item. Decrement it (×3 items count down), then relay the same line
  with the sender guid to players within 150 yd, excluding the sender. Reuse `RelayState`'s
  pattern with its own rate table, e.g. ≥ 250 ms per player. Juiced = wumpa ≥ 10 is known
  server-side, so append it in the relay.
- **Snapshot.** `W\tB\t<course>\t<downMaskHex>`, sent on `R\tC` (race start) and when someone
  claims near a course. One line per course: 32 crates make an 8-hex-char mask.
  - Crate positions go to clients either in a `W\tP\t<course>\t<i>:<x>:<y>:<z>;...` line,
    chunked to ≤ 200 bytes (~10 crates per line), or, simpler, in a client-side copy of the
    crate layout. The server copy is still needed for validation, and two hand-kept copies is
    what `race.rs` COURSES already does for starts.

**Client-authoritative (relayed)**

- **Projectile simulation** by the firer, and by every receiver from the `F` line. The
  missile's homing target is in the line. Receivers simulate the same deterministic path from
  the same start, so no per-tick projectile traffic is needed. Optionally add
  `W\tX\t<seq>\t<x>,<y>,<z>` for an early detonation: hit wall, shield or mine trigger.
- **Hits** (`W\tH\t<shooterGuidLow>\t<seq>\t<effect>`): sent by the **victim** when its local
  sim hits it (spin-out, blast, tumble). The server checks that `seq` was fired by the shooter
  recently (keep the last ~8 `F` per shooter with timestamps and positions) and that the
  victim is within plausible distance. It then relays to the 150 yd set so others play the
  spin-out and explosion. A hit can also feed rewards (XP, skill) from the shooter's side.
- **Dropped mines** (TNT, Nitro, potion) are fires with zero velocity, so each is just an `F`
  line. A mine's lifetime and owner live in the relayed `seq`. Whoever drives into it sends
  `H`. To stop a mine being hit twice, the server marks `(shooter, seq)` consumed on the first
  accepted `H` and relays `W\tX`.
- **Shield, Aku Aku, clock, turbo.** These are self-buffs plus visuals. Carry them as new flag
  bits in `S` (bits 4+: shield up, juiced shield, mask, clock-slowed). That costs no new
  traffic and already reaches everyone in 150 yd every 100 ms.
  - The clock affects everyone else. Treat it as an `F` with item 8; receivers apply the slow
    to themselves.

**Implementation sketch**

- **Server:**
  - add `else if (body.compare(0, 2, "W\t") == 0) HandleWeapon(player, body);`;
  - split fields the way `HandleRace` does; require the Summon Kart aura;
  - keep a per-player `s_lastWeapon` timer per sub-verb;
  - clean `held` and `wumpa` on logout. Note that `s_lastLine`, `s_lastState` and `s_races`
    are never pruned per player today either.
- **Client:**
  - a `kart/weapons.rs` with an `on_chat` for `KARTCRAFT\tW\t`, registered in `kart/mod.rs`
    next to `remote::on_chat` and `race::on_chat`;
  - send through `profession.send(...)`;
  - feed `HudExtras { item, wumpa }`;
  - draw from `weapons.json` like `kartcraft::effects` draws `effects.json`.
- **Keep every line ≤ 200 bytes** (hard 255). Use guid lows rather than names. Use integer
  yards ×10 for positions, as `S` does for its fixed-point fields.
- **To verify before relying on it:** whether vmangos processes `CMSG_MESSAGECHAT` on
  map-update worker threads.
  - Kartcraft's `static` maps are unguarded, which is fine on one thread.
  - Server-owned crate state and `held` maps written from several maps' threads would need a
    mutex.
  - Check `Opcodes.cpp`'s processing flag for `CMSG_MESSAGECHAT` in the vmangos tree the
    server build uses.
