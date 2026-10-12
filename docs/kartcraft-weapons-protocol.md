# Kartcraft weapons: the protocol

How weapon crates, Wumpa fruit, items, shots and hits travel between the client and the vmangos
server. CTR's own rules are `docs/ctr-weapons-spec.md`. The transport (addon lines, relays,
limits) is `docs/ctr-weapons-assets-and-net.md`, part B.

## Who decides what

- **The server owns:** where the crates are, whether each is taken and when it comes back, the
  roulette result (CTR's weighted tables, by race position), what each racer holds and how many
  shots are left, and each racer's Wumpa count. A shot is accepted only while its item is held.
- **Clients own the physics:** the shooter's client and every other client simulate a relayed
  shot from the same start (`ctr-weapons`). The **victim's** client decides it was hit, reports
  it, and applies the reaction to its own kart (`ctr-kart`). The server checks that the shot
  exists and relays the hit, so everyone shows the reaction and removes the projectile.

Weapons exist only **inside a race**. Crates belong to a course and are live while a race on it
runs. Only that race's racers can take them, and shots and hits are relayed to that race's
racers. A time trial (`R\tTT\t<id>`, `ctr-weapons-assets-and-net.md` B3) has none: no layout is
sent, every `W` line from its racer is refused, and the client binds no weapons to it.

## Lines

Each line is `KARTCRAFT\tW\t<verb>\t<fields...>`. Fields are `\t`-separated; lists inside a field
are `,`-separated. A whole line stays under 200 bytes. Positions are WoW world coordinates in
yards with 2 decimals; yaw is in radians with 3 decimals. Guids are decimal low guids.

Item ids are CTR's (`ctr-weapons-spec.md`, the item table): 0 Turbo, 1 Bomb, 2 Missile, 3 TNT
(Nitro when juiced), 4 Beaker, 6 Shield, 7 Mask, 8 Clock, 9 Warp orb, 10 Bomb ×3, 11 Missile ×3.
15 is none.

Hit kinds: `S` spin-out, `B` blasted, `Q` squished, `T` TNT landed on the head.

### Server → racers when a race starts

`W\tL\t<first>\t<crate>;<crate>;...`, with as many lines as needed, starting at index `<first>`.
Each crate is `<kind>,<x>,<y>,<z>`. Kind is `Q` for a weapon crate ("?"), `F` for a fruit crate,
`W` for a single Wumpa fruit. Indices count from 0 in send order. The client draws them all.

### Client → server: take a crate

`W\tC\t<index>`. The server accepts it when:
- the sender is a racer in a running race on that course;
- the sender is within 12 yd of the crate (server-side position, which trails a fast kart);
- the crate is up and has regrown (533 ms after its `U`);
- and, for `Q`, the sender holds nothing and has no shots left.

For a `Q` crate the item is still rolled even if the sender already holds one: CTR breaks the
crate and gives nothing.

Every racer gets `W\tX\t<index>\t<guid of taker>`, and the crate disappears. It comes back after
its 30-frame cooldown (1000 ms) as `W\tU\t<index>` and regrows over 16 frames, 46 in all
(`RB_Crate.c:48-75`, `:125`). Single fruit never come back.

The taker also gets:
- for `Q`: `W\tI\t<item>\t<shots>`. The client spins the roulette for 90 frames (Fire stops it
  early after 20), then shows that item. The server accepts shots of it from 600 ms after the
  claim (20 frames).
- for `F` and `W`: `W\tF\t<wumpa total>`. A fruit crate gives 5–8, a single fruit 1, capped at 10.

A refused claim gets no answer.

### Client → server: a shot broke a crate

CTR's crates break for whatever reaches them (`ctr-weapons`' `boxes`; `RB_Crate.c:123-131`): a
bomb, missile or shield bomb touching one or bursting within its blast (320 lu, 512 juiced, a
missile's 128) of it, a warp orb touching one, and a TNT, Nitro or beaker dropped on one. A TNT or
Nitro going off breaks none, and single fruit are for karts alone. The shooter's client decides it
and sends `W\tK\t<shot>\t<index>`, `shot` from its own counter. The server accepts it when:
- the sender has that shot, relayed in the last 30 s, and it breaks crates: a bomb, missile or
  warp orb (not dropped), a shield launched with `Z` (not one put up), or a dropped TNT or beaker;
- the crate is a `Q` or `F` crate, up and regrown;
- it is within reach of where the shot left (the `S` or `Z` place): a dropped one 12 yd, others
  as far as the warp orb's 112 lu a frame (92 yd/s) since the relay, plus 16 yd (a juiced bomb's
  blast, the crate and slack).

Every racer gets `W\tX\t<index>\t<guid>`, as for a claim, and the crate comes back on the same
1000 ms cooldown (`RB_Crate.c:125`). A bomb, missile or shield bomb gives its shooter the crate
as their kart's touch would (`RB_Crate.c:152-226`, `:315-355`): `guid` is the shooter's, and the
`I` or `F` follows as for a claim (none once they finished). A warp orb or a mine gives no one
anything: `guid` is 0. A refused break gets no answer.

### Client → server: fire

`W\tS\t<shot>\t<x>,<y>,<z>,<yaw>\t<target guid or 0>\t<flags>`

- `shot` is the sender's own counter, 1–65535.
- Flag `b` means fired backwards (bomb or missile, the stick held back); `d` means dropped behind
  (TNT, Nitro, Beaker).

The server checks the held item and shots left, takes one shot, and decides juiced (Wumpa ≥ 10
now). Every racer, the sender included, gets
`W\tS\t<guid>\t<item>\t<juiced 0|1>\t<shot>\t<x>,<y>,<z>,<yaw>\t<target>\t<flags>`.

- Turbo, Shield, Mask and Clock relay the same way. Each client applies Turbo, Shield and Mask
  to the shooter's kart (effects, plus the shooter's own physics), and Clock to every kart
  except the shooter's.
- Warp orb: the server fills `target` with the race leader (the next racer if the shooter leads).

### Client → server: fire again

CTR's Fire with a bomb rolling sets it off, and Fire with a shield up launches it as a shield
bomb (`ctr-weapons-spec.md`, 3.6).

- `W\tD`: detonate. Accepted while the sender has a bomb shot (Bomb or Bomb ×3) relayed in the
  last 30 s that has not ended. Every racer gets `W\tD\t<guid>`, and the sender's rolling bomb
  goes off where each client has it.
- `W\tZ\t<shot>\t<x>,<y>,<z>,<yaw>`: launch the shield. `shot` is from the sender's own counter,
  as for `S`. Accepted once per shield, while the sender has a relayed Shield up: within 13 s of
  it (CTR's 12 s and a second's slack), or any time for a juiced (blue) one. Every racer gets
  `W\tZ\t<guid>\t<shot>\t<x>,<y>,<z>,<yaw>`; the shield leaves the shooter's kart and a shield
  bomb rolls straight on from there. It is a shot like any other: hits name it.

A refused `D` or `Z` gets no answer.

### Client → server: I was hit

`W\tH\t<shooter guid>\t<shot>\t<kind>`.

The server checks that the shot was relayed in the last 30 s. For a missile or warp orb, the shot
must also not have hit anyone yet; explosions may hit several racers. It then takes Wumpa (S −1,
B −3, none while the victim is masked) and relays to every racer:
`W\tH\t<victim guid>\t<shooter guid>\t<shot>\t<kind>\t<wumpa left of victim>`.

Every client removes the projectile (or triggers its explosion) and shows the reaction on the
victim's kart. A hit by a dropped item ends that item.

Self-inflicted hits use shooter guid = own guid, shot 0, for example a TNT going off on your own
head or driving into your own Nitro. They are always accepted and only relayed.

### Client → server: Wumpa lost by no one's hit

`W\tP\t<why>`: `S` a powerslide spun out (CTR's SpinFirst, 1 Wumpa, `VehPhysProc.c:2868-2871`;
a hit's spin-out is paid with the hit), `M` Aku Aku put the kart back after a fall (the mask
grab, 2 Wumpa, `VehStuckProc.c:648-651`). The server takes it (none while masked) and answers
the sender alone with `W\tF\t<wumpa total>`. Sent only inside a race, as CTR takes it only there.

### Server → racers: a rain cloud

A spin-out (`S`) reported against a juiced beaker shot (a red beaker) puts CTR's rain cloud on
the victim (`RB_RainCloud_Init`, `231/RB_MaskShieldCloud.c:568-641`). After the `H` relay every
racer gets `W\tR\t<victim guid>\t<effect>\t<locked 0|1>`. The cloud lasts 7680 ms from that line.
Effects are CTR's: `0` slow (top speed less `DamagedSpeed`), `1` item roll, `2` ice (any ground is
ice), `3` no hop.

- A first cloud is `1` if the victim holds an item with a shot left, else `0`. Under a live cloud,
  another red beaker restarts the time and draws the effect anew: `(MixRNG % 400) / 100`.
- `locked 1`: the victim's held item is gone into the roulette. The victim's client shows it
  spinning, and Fire does not stop it. The server takes no shots from the victim. A weapon crate
  gives a racer under a `1` cloud nothing, locked or not (`RB_Crate.c:185-191`).
- When the cloud goes, a locked roulette is rolled anew as a crate's would be, and the victim gets
  `W\tI\t<item>\t<shots>`. Its shots are accepted at once. A redraw that leaves `1` rolls it at
  once, too: CTR's roll timer, no longer held at 5 frames, runs out.
- The victim's own mask (an `S` of item 7) blows the cloud away, with no roll.

### Client → server: I was rammed

`W\tB\t<attacker guid>\t<how>`, where `how` is `M` mask, `S` shield or `T` turbo
(`VehPhysCrash_Attack`, `Vehicle/VehPhysCrash.c:199-252`). The victim's client decides it as it
decides a weapon hit, from the karts touching and what the other kart wears there: a mask or a
shield blasts, and a turbo item burning, at a closing speed over 0xA00, squishes a kart with no
reserves. It sends the line only when the hit took (no mask, shield or invincibility of its own),
at most once a second per attacker.

The server accepts it when the attacker is another racer of the race, within 20 yd of the victim
(both server positions trail their karts), and is armed:
- `M`: its mask is up (as for the Wumpa rule below).
- `S`: it has a relayed Shield up (as for `Z`). The ram spends it: a later `Z` is refused.
- `T`: it fired a Turbo in the last 3 s (CTR's 2400 ms of reserves, and slack).

It takes Wumpa as for the hit (`M` and `S` −3 as a blast, `T` −1 as a squish, none while the victim
is masked) and relays to every racer:
`W\tB\t<victim guid>\t<attacker guid>\t<how>\t<wumpa left of victim>`.

Every client shows the reaction on the victim's kart. A shield ram takes the attacker's shield off.
The attacker's own side is its own client's, from the same touch: its shield pops, and a turbo ram
makes it high-jump (`FORCED_JUMP_HIGH`, three times the hop's force).

### Limits

At most 20 `W` lines a second from one sender; the rest are dropped. At most 12 live shots per
race (CTR's 12-projectile pool); an extra fire is refused (no relay).

## How the server reads this

Notes on `setup/vmangos/src/Kartcraft.cpp`'s `W` handling, where this page leaves room:

- **Layout.** At `R\tS` each segment between consecutive checkpoints (and from the last back to
  the first on a lap course) gets, in send order: a fruit crate a quarter of the way, a row of 4
  weapon crates across the path halfway (4 yd apart), and 3 single fruit three quarters along
  (4 yd apart). `z` is the ground there (terrain, then a vmap floor just above it).
- **A `Q` crate taken while holding an item** is still taken (`X`, and `U` later) but gives no
  `I`: CTR breaks the crate and gives nothing.
- **Wumpa lost per hit kind:** `S` and `Q` 1, `B` 3, `T` none (the blast comes later as its own
  hit). Self-inflicted hits skip the shot check but still cost Wumpa.
- **The item in the `S` relay** is the held id, so Bomb ×3 and Missile ×3 relay as 10 and 11.
- **Live shots** (the 12-shot pool) are projectiles (bomb, missile, TNT, beaker, warp orb, ×3
  items) relayed in the last 30 s that have not ended: a missile or warp orb that hit, or a dropped
  item that was hit. Turbo, Shield, Mask and Clock do not count, nor does a shield bomb (it
  is recorded as a Shield shot): the shield it was already took its place.
- **A shield up** is the racer's last relayed Shield, until a `Z` launches it, a new race starts
  or 13 s pass (a juiced one has no time limit). A shield popped by a hit is not reported, so a
  `Z` after a pop inside those 13 s is still relayed; the shooter's own client sends none.
- **Roulette field size** is the race's racer count. With 1 or 7 racers CTR has no table and rolls
  uniformly over ids 0–11 (Spring becomes Turbo). The lap-1 downgrade uses the racer's lap, so on
  a sprint (no laps) Race4 is always downgraded to Race3.
- **Respawns** are applied on the next Kartcraft line from anyone (karts send state ten times a
  second), so a `U` may come up to about 100 ms after the 1000 ms. A rain cloud's end, and the
  `I` of the roulette it held, come the same way.
- **A red beaker** is a Beaker shot relayed juiced. Only an `S` hit naming it brings the cloud.
- **A turbo ram** counts from the attacker's last relayed Turbo. The server does not know a
  kart's reserves, so a drift boost does not count, and braking out of the turbo is not seen.

## How the client reads this

`kartcraft::weapons` (`crates/kartcraft/src/weapons/`) is the client half. `Weapons` takes the
server's lines (`wire::Down`, parsed from the addon text) and queues the racer's (`wire::Up`);
it steps at 30 Hz inside the kart's session, after each kart tick, over the session's own
collision and lu origin. The client (`benilla-app/src/kart/weapons.rs`) binds it to the race it
is in, sends its lines at no more than 20 a second, and clears it when the race ends for us, we
leave it or we leave the kart. The offline demo plays against `weapons::referee::Referee`, the
server's rules in-process, so it runs the same code.

- **Crates.** `L` lays them out (index from `first`); a full-grown crate within 2 yd of our kart (the
  server allows 12) breaks at once on our screen, as CTR breaks it the frame it is touched, and
  is claimed with `C`; with no answer in 30 frames the claim was refused and the crate is back,
  regrowing. `X` breaks it for everyone; taken by us, a `Q` crate starts the roulette at once if we hold
  nothing, and `I` sets its result, shown when the 90-frame spin ends (Fire stops it after 20).
  A roulette with no `I` 60 frames past its end is dropped. `U` regrows the crate over 16
  frames. `F` sets the Wumpa total, which the HUD counts up one fruit every 5 frames.
- **Crates our shots break.** Each frame our live bombs, missiles, shield bombs and warp orbs
  are tested against the full-grown crates (a 48 lu box, half CTR's), and so are our bursts and
  a TNT or beaker we drop; a crate one reaches breaks at once on our screen and is sent as `K`,
  waiting for the `X` as a claim does. The shot flies on. Another racer's shots break crates here
  on the server's `X`; an `X` naming no one (0) starts no roulette.
- **Shots.** Fire sends `S` with our position and facing (WoW yaw), the missile lock-on (the
  nearest other racer inside our camera's view) for a bomb or missile, `b` with the stick back,
  `d` for TNT and beakers. Nothing launches until the relay, ours included. A relayed shot puts
  Turbo, Shield and Mask on the shooter's kart (ours through `ctr-kart`, another's on its ghost),
  Clock on every other kart, and the rest into the `ctr_weapons::Field` from the relayed spot.
- **Hits.** A field hit on our kart goes through `Kart::hit`; if it takes (no mask, shield or
  post-pop invincibility) we send `H`. A TNT that blasts us as it lands reports `B`. Hits on
  other karts are theirs to report, except that a shield we see on them pops here as it does
  there. A relayed `H` sets our Wumpa when it is ours, shows the reaction on another's kart, and
  ends the shot (a juiced warp orb flies on).
- **Other karts** each carry a ghost `ctr_kart::Kart` standing on a flat plane: their shield,
  mask, TNT, tumble and squish, drawn on the kart the client already places. Their spin comes
  with their relayed state.
- **TNT on a head** going off is a self-inflicted `B` (our guid, shot 0). Another client seeing
  that line from a racer it saw carrying a TNT shows the blast there and, if its own kart is
  within 320 lu, blasts it and reports that as its own self-inflicted `B`: a TNT on a head is no
  longer a shot the server knows.

- **Fire again.** With our bomb rolling, Fire sends `D`, and the bomb goes off on the relay, ours
  included. With our shield up, Fire sends `Z` from our kart and takes the shield off it at once;
  the shield bomb launches on the relay, and another's shield leaves their ghost then.

- **Rain clouds.** `R` hangs the cloud over the victim's kart for 240 frames. Over ours it is
  `ctr-kart`'s cloud effect (slow, ice or no hop); `locked` puts our item into the roulette
  (`Held::cloud_lock`) until the `I`. A roulette still locked when our cloud goes waits 60 frames
  more for the `I`, as a crate's does. A field's red-beaker outcome brings no cloud by itself.
- **Rams.** Each nearby kart is bumped with what its ghost wears: its mask and shield, and the turbo
  item for 75 frames after its relayed Turbo (`Weapons::arm`). `ctr-kart` decides the ram both
  ways in its kart collision. A ram on us that took is sent as `B`. Our side, a spent shield or the
  high jump, is applied at once. A demo dummy we ram is blasted or squished here and reports it.
  `B` relayed shows the victim's reaction on its ghost, and a shield ram takes the attacker's
  shield, ours included, if it is still up.

Not on this wire: a TNT thrown off by hopping (it goes off where it lands, without damage).
Weapons are stepped only inside our session's collision, 120 yd round our kart: a shot farther
out finds no ground and goes off.
