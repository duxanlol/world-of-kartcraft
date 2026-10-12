# ctr-assets: the kart from the player's own CTR disc

`crates/ctr-assets` converts the player's own *Crash Team Racing* disc image (PS1, NTSC-U,
SCUS-94426) into `kart.json`: one racer's kart **without its driver**, its wheels, anchors for
seating a WoW character, and decoded textures. No game data is in the repository; setup runs
the converter on the player's machine. The disc image is only read.

```sh
cargo run -p ctr-assets --release --bin ctr-extract -- "<disc>.bin" <out_dir> [--kart <name>] [--audio] [--effects] [--weapons] [--hud] [--music] [--all]
```

- writes `<out_dir>/kart.json` (default kart: `crash`);
- `--kart` picks another racer's kart: `crash cortex tiny coco polar pura pinstripe roo joe pen fake`;
- `--audio` also writes the kart, weapon and pickup sound effects to `<out_dir>/audio/` (see
  [Audio](#audio));
- `--effects` writes `<out_dir>/effects.json` (turbo flames, exhaust smoke, skid marks,
  particles; see [Effects](#effects));
- `--weapons` writes `<out_dir>/weapons.json` (weapon, crate and Wumpa fruit models; see
  [Weapons](#weapons));
- `--hud` writes `<out_dir>/hud.json` (the 1P race HUD; see [HUD](#hud));
- `--music` renders the race music to `<out_dir>/music/` (see [Music](#music));
- `--all` writes `<out_dir>/kart-<name>.json` for every kart, the audio, effects, weapons, HUD
  and music.

Raw `.bin` (2352-byte sectors) and cooked `.iso` (2048) images both work; the `.cue` is not
needed. Extraction takes well under a second; each file is ~0.3 MB.

The code ports format knowledge from [CTR-tools](https://github.com/CTR-tools/CTR-tools)
(GPL-2.0) and the CTR-native decompilation (GPL-3.0), so the crate is GPL-3.0-or-later
(see the header of `src/lib.rs`).

## Formats parsed

### Disc (`disc.rs`)

The image is Mode 2: each 2352-byte raw sector is 12 sync bytes, a 4-byte header and, for
Form 1 data sectors, an 8-byte XA subheader followed by 2048 user bytes (offset 24). The
layout is detected from sector 16 (the ISO 9660 primary volume descriptor, `CD001`). The ISO
9660 root directory is walked to find `BIGFILE.BIG`.

### BIGFILE (`big.rs`)

`u32 0`, `u32 count`, then `count` × `{u32 offset_in_sectors, u32 size_in_bytes}`. Entries
are unnamed; the NTSC-U indices used (608 entries on this disc):

| index | file |
|---|---|
| 242 + id | `models\racers\hi\<racer>.ctr` (id order: crash, cortex, tiny, coco, ngin, dingo, polar, pura, pinstripe, papu, roo, joe, ntropy, pen, fake, oxide) |
| 258 | `packs\shared.vrm`: VRAM with every racer's textures and the tire sprites |
| 259 | `packs\shared.mpk`: icon groups (group 0 `tireAnim`) |

### Model `.ctr` (`ctr.rs`)

Wrapped as `{u32 size, data[size], u32 map_size, u32 pointer_map[]}`; pointers in `data` are
offsets from its start.

```text
Model       { char name[16]; s16 thread; s16 num_meshes; u32 ptr_meshes }
ModelHeader { char name[16]; i32; s16 lod; u16 flags; s16 scale[3]; s16 pad;
              u32 ptr_commands; u32 ptr_frame (0 if animated); u32 ptr_tex_layouts;
              u32 ptr_colors; u32; u32 num_anims; u32 ptr_anims; u32 anim_tex }   // 0x40 bytes
ModelFrame  { s16 pos[3]; s16 pad; u8[16]; u32 vertex_offset (0x1C); u8 verts[n][3] }
ModelAnim   { char name[16]; u16 frames (bit 15: half-rate); s16 frame_size;
              u32 ptr_deltas (0 = raw bytes); ModelFrame frames[] }
TextureLayout { u8 u0,v0; u16 clut; u8 u1,v1; u16 tpage; u8 u2,v2,u3,v3 }       // 12 bytes
```

- **Vertices** are bytes. Model-space position (CTR model units) =
  `(byte + frame.pos) * scale / 4096`, with the stored byte order `[x, z, y]`
  (`RenderBucket_PackModelVertexXY`).
- **Compressed frames** (racer animations): per vertex a shared delta word
  `x_base:7(<<1) | z_base:8 | y_base:8 | x_bits:3 | z_bits:3 | y_bits:3`; each component reads
  `bits+1` signed bits MSB-first from the frame's little-endian 32-bit word stream and
  accumulates mod 256; `bits == 7` replaces the accumulator. Components arrive x, z, y.
- **Commands**: `u32 scratch_color_count`, then `u32` commands to `0xFFFFFFFF`, bits
  `s l n d k v ? ? | stack:8 | color:7 | tex:9`. Each command loads the next vertex (or, with `v`,
  re-reads stack slot `stack`) into a 4-deep FIFO; `s` restarts a strip; from the third vertex
  on each command emits the triangle FIFO[1..3]; `l` first replaces FIFO[1] by FIFO[0]
  (turning the strip into a fan); the first triangle of a strip uses the strip's first
  command for texture/flags. `d` = back-face culled, `n` = flipped winding. Commands whose top
  16 bits are zero only load colours. `tex` 0 = untextured, else `tex_layouts[tex-1]`; corner
  `i` of a triangle uses `uv[i]`. Colours are `u32` RGBX.

### VRAM `.vrm` and TIM (`vram.rs`)

A stream (`u32 0x20`, then `{u32 size, TIM}`… until size 0) or one TIM. TIM:
`u32 0x10, u32 flags (bit 3 = CLUT)`, optional CLUT block, image block; blocks are
`{u32 len, u16 x, y, w, h, u16 data[w*h]}` in VRAM halfword units and are blitted into a
1024×512 16-bit VRAM. `shared.vrm` fills (896,0)-(1023,255) and the strip (0,216)-(511,263),
where every racer's texture and CLUT live.

Texture decode from a `TextureLayout`: page at `x = (tpage & 15) * 64`, `y = (tpage >> 4 & 1) * 256`;
depth `tpage >> 7 & 3` (0 = 4bpp, 1 = 8bpp, 2 = 15bpp); CLUT at `x = (clut & 63) * 16`,
`y = clut >> 6`. 4bpp packs 4 texels per halfword (low nibble first), 8bpp 2. Colours are
15-bit `0bSBBBBBGGGGGRRRRR`, expanded 5→8 bits. PSX transparency rules: a texel of exactly
`0x0000` is fully transparent (alpha 0); a texel with the STP bit set gets alpha 128 when the
primitive is semi-transparent (tpage blend mode ≠ 3), otherwise 255. All kart textures use
blend mode 3 (opaque), so kart surfaces are `"opaque"` with cut-out (alpha 0) texels only.

### Model pack `.mpk` icons (`mpk.rs`)

Same pointer wrapper; `data[0]` points to the icon pack
`{u32 num_icons, u32 ptr_icons, u32 num_groups, u32 ptr_groups}`; icons are
`{char name[16]; i32 index; TextureLayout}`; groups `{char name[16]; s16 id; s16 n; u32 icons[n]}`.
Group id 0 (`tireAnim`, `tire00`…`tire16`, 32×32 each) is what `VehBirth_TireSprites` gives
every driver.

## Kart vs driver separation (`kart.rs`)

Each racer's `hi` model is one animated mesh holding kart and driver (crash: 272 triangles,
87 of them kart). Two independent signals separate them:

1. **Shared geometry.** All racers sit in the same kart geometry (only colours/CLUTs differ).
   For each triangle we count how many of the other 13 racers have vertices at all three of
   its corners (±3 model units per axis, anywhere in their mesh, so different triangulation
   still matches). Kart corners match nearly everyone; driver corners match nobody.
2. **Animation.** In the 21-frame `turn` animation the driver leans and steers while the kart
   body does not move at all: its vertices are bit-identical in every frame.

A triangle is kart when it is static and shared by ≥ 3 racers, or animated and shared by
≥ 8 (a rear engine piece that shakes), or animated, in front of the seat and shared by ≥ 5
(the steering wheel, which sits a little closer or further for big and small drivers). The
exported frame is `turn[10]` (steering centred). Fake Crash is not compared with Crash (same
driver); Oxide (hovercraft) is not used.

Each result is checked for completeness (surface area of nose, hood/steering column, floor and
rear engine, in model units²). N. Gin, Dingodile, Papu Papu and N. Tropy have the hood fitted
to their body and fail it, so they are not offered as `--kart` choices.

To see the separation, render the full model with kart triangles green and driver red:

```sh
CTR_CLASSIFY_COLORS=1 cargo run -p ctr-assets --release --example render_kart -- kart.json out.png --disc <disc>.bin
```

## Wheels

The wheels are **not** in the mesh. `DrawTires_Solid` draws each as a textured quad
(`POLY_FT4`) at fixed instance-local positions, sized by `driver->wheelSize` (0xCCC), tinted
with `0x808080` (neutral), choosing one of the 17 `tireAnim` sprites by view angle:

- centres `x = ±0x90`, `y = 0x40`, `z = 0xC7` (front) / `-0x60` (rear), times the instance
  scale, in quarter world units; the quad's half-size is `wheelSize * 4096 >> 18`.
  In `kart.json` units: centres `(±0.5625, 0.25, 0.777)` front and `(±0.5625, 0.25, -0.375)`
  rear, radius `0.25` (the wheels touch `y = 0`).
- the quad is a camera-facing billboard (side `2 * radius`) whose axes are
  `cross(view, axle)` and `cross(view, that)`, so it rolls with the axle; the front wheels'
  axle is turned by the steering angle.
- the sprite is picked by how much of the wheel's side faces the camera: roughly
  `|cos(angle between view direction and axle)| * 128` through a non-linear table
  (`sDrawTiresSpriteIndexTable` in CTR-native `DrawTires.c`): `tire00` = tread seen edge-on,
  `tire16` = side seen straight on (full disc with hubcap). A client can do the same
  (billboard + sprite pick), or draw each wheel as a disc of radius `radius` facing ±X with
  `tire16` (what the verification render does), plus a dark cylinder for the tread if wanted.

## Coordinates and units

- **Right-handed, +Y up, +Z forward (the kart's nose), +X = the kart's left** (glTF
  convention). Verified: Cortex's forehead "N" reads correctly from the front, and culled
  renders show outer surfaces.
- **Meters**: `position_m = model_units / 64`. A model unit is
  `(byte + frame.pos) * scale / 4096`; the game additionally scales racer instances by 0.8,
  which we drop. The kart comes out 1.75 m long, 0.88 m wide (body; wheel centres 1.125 m apart),
  0.81 m tall to the exhaust tips, wheels 0.5 m in diameter — cartoon go-kart proportions.
  `units.ctr_units_per_meter` records the 64.
- **Origin**: the CTR driver instance origin: centred left/right, on the ground plane
  (wheel bottoms at `y = 0`), between the axles (`z = 0` is 0.38 m ahead of the rear axle).
- **Front faces wind counter-clockwise**; `normal` is the face normal.

## `kart.json` schema

```jsonc
{
  "format": "ctr-kart", "version": 1,
  "kart": "crash",
  "source": { "game": "...", "model": "crash_hi (crash)", "frame": "turn[10]",
              "triangles_total": 272, "triangles_kart": 87 },
  "units": { "length": "meters", "ctr_units_per_meter": 64.0, "ctr_instance_scale": 0.8 },
  "axes": { "up": "+Y", "forward": "+Z", "left": "+X", "handedness": "right",
            "front_face": "ccw", "origin": "..." },
  "bounds": { "min": [x, y, z], "max": [x, y, z] },          // kart mesh only, meters
  "anchors": {
    "seat": [0, 0.254, -0.029],          // where a seated driver's hips (pelvis) go
    "steering_wheel": [0, 0.615, 0.401], // centre of the steering-wheel ring
    "exhaust": [[0.281, 0.875, -0.875], [-0.281, 0.875, -0.875]]  // pipe outlets (left, right)
  },
  "wheels": [ { "name": "front_left", "center": [x, y, z], "radius": 0.25, "steers": true }, ... ],
                                          // front_left, front_right, rear_left, rear_right
  "wheel_sprites": { "textures": [14, ..., 30],   // indices of tire00..tire16 in "textures"
                     "rule": "...", "tint": [1, 1, 1, 1] },
  "surfaces": [ {
      "texture": 0,                      // index into "textures", or null (untextured)
      "double_sided": false,             // false: cull back faces
      "blend": "opaque",                 // or "blend" (PSX 50/50 for alpha-128 texels)
      "vertices": [ { "position": [x, y, z], "normal": [x, y, z],
                      "uv": [u, v], "color": [r, g, b, a] } ],
      "indices": [0, 1, 2, ...]          // triangle list
  } ],
  "textures": [ { "name": "page960_0_clut496_257_4bpp", "width": 16, "height": 16,
                  "rgba": [r, g, b, a, ...] } ]   // RGBA8, row-major, top row first
}
```

- `uv` is normalized to the texture (`(texel + 0.5) / size`, v down, top row = v 0); use
  nearest filtering and alpha-test `a == 0` (cut-out texels).
- `color` is the PSX Gouraud vertex colour. On **textured** surfaces it is a modulation
  factor: `final = texel.rgb * color.rgb` with `1.0` = neutral (PSX `0x80`); values up to ~2
  brighten. On **untextured** surfaces it is the colour itself (`0..1`). PSX models are
  unlit: lighting is baked into these colours, so draw unlit.
- Each texture is one page+CLUT combination cropped to the texels the kart uses; the 17 tire
  sprites follow them.

## Verifying visually

`examples/render_kart.rs` is a small software rasterizer (z-buffer, perspective-correct
nearest texturing, back-face culling, alpha cut-out) that reads `kart.json` and writes a PNG:
row 1 the exported kart from four views with the wheel sprites and anchor markers (seat
magenta, steering wheel cyan, wheel centres yellow), row 2 (with `--disc`) the same racer's
full model, then the 17 tire sprites. `--sheet a.json b.json … out.png` renders one view
per kart.

```sh
cargo run -p ctr-assets --release --example render_kart -- kart-data/kart.json kart-data/render-crash.png --disc <disc>.bin
```

## Tests

`cargo test -p ctr-assets` runs the parser unit tests (ISO 9660 records and sector layout,
bigfile TOC, command fields, strip assembly, delta decompression, bit reader, TIM/VRM
4bpp/8bpp decode and transparency, classification). With `CTR_DISC=<disc>.bin` set,
`tests/disc.rs` also extracts every kart, the audio and the weapons from the real disc and checks the
results. The ADPCM decoder, loop handling, WAV writer, HOWL table/bank parser and the
distortion factors (against the game's tables) have unit tests.

## Audio

`--audio` reads `SOUNDS\KART.HWL` (the HOWL sound bank, ~12 MB, outside the bigfile), decodes
the PS-ADPCM samples the game plays for kart events and writes mono 16-bit WAVs at their
native rates (no resampling) into `<out_dir>/audio/`, plus `audio/sounds.json` (rate,
length, loop, peak, HOWL ids, the game's volumes and the decomp reference for every file).

### HOWL (`audio.rs`)

```text
HowlHeader { "HOWL"; u32 version (0x80); u32; u32; u32 num_spu_addrs; u32 num_other_fx;
             u32 num_engine_fx; u32 num_banks; u32 num_seqs; u32 header_size }   // 0x28
SpuAddr    { u16 runtime_addr; u16 size / 8 }                 // per SPU sample (528)
OtherFX    { u8 flags; u8 volume; u16 pitch; u16 spu_index; u16 duration }       // 258
EngineFX   { u8 flags; u8 volume; u16 pitch; u16 unk; u16 spu_index }            // 20
u16 bank_sector[num_banks]; u16 seq_sector[num_seqs]          // * 2048
Bank       { u16 count; u16 spu_index[count]; pad to 2048; raw ADPCM of each, in order }
```

Layouts follow CTR-native `include/namespace_Howl.h`. CTR-tools reads EngineFX's sample
index at offset 4; the decomp's offset 6 is right, and it groups the 20 engine entries into
5 classes × 4 player slots that each share one sample. A sample's natural rate is
`pitch * 44100 / 4096` (SPU pitch 0x1000 = 44.1 kHz). OtherFX `flags & 2` marks a looped
sound. `OtherFX_Play*` / `OtherFX_RecycleNew` index OtherFX by sound id; `EngineAudio_*`
index EngineFX by `engine_class * 4 + player`.

### PS-ADPCM and loops

Each 16-byte block holds 28 samples: byte 0 = `filter << 4 | shift`, byte 1 = flags, then
14 bytes of nibbles (low nibble first).
`s = (nibble << 12 >> shift) + ((s1 * F0[f] + s2 * F1[f] + 32) >> 6)` with
`F0 = 0, 60, 115, 98, 122` and `F1 = 0, 0, -52, -55, -60`, clamped to i16. Block flags:
bit 2 = loop start, bit 0 = end, bit 1 = repeat (end + repeat jumps back to the loop start).

Looped sounds (engine, skid, turbo) are written as the **steady loop only**: the region from
the loop-start block to the end block, decoded a second time with the ADPCM history the SPU
carries over from the loop end. Repeating the file is then exactly what the console plays.
Measured seam jumps: engine 9, skid 166, turbo 293, against mean sample steps of 2886, 9014
and 5683. Loops shorter than half a second are repeated (the engine loop is 952 samples,
78 ms), and a WAV `smpl` chunk marks the whole file as one infinite forward loop. One-shots
are written in full with trailing silence trimmed.

### Files and where the ids come from (CTR-native)

| file | HOWL id | rate | length | source in the decomp |
|---|---|---|---|---|
| `engine.wav` (loop) | EngineFX 0 (BALANCED, player 1) → SPU 0, pitch 1140 | **12274 Hz** | 0.54 s | `HOWL/HOWL_Engine.c` `EngineSound_Player`: `EngineAudio_Recalculate(engine*4 + driverID, …)`; Crash `engineID = BALANCED` (`zGlobal_DATA.c` `MetaDataCharacters`; enum `EngineClass` in `namespace_Vehicle.h`) |
| `skid.wav` (loop) | OtherFX 0x11 → SPU 15 | 8990 Hz | 1.07 s | `Vehicle/VehEmitter.c`: `OtherFX_RecycleNew(.., terrain->skidSound, ..)`; asphalt (also stone, wood, track) `skidSound = 17` (`zGlobal_DATA.c` `MetaDataTerrain`) |
| `turbo.wav` (loop) | OtherFX 0x0E → SPU 9 (looped flag) | 22050 Hz | 0.65 s | `Vehicle/VehTurbo.c`: `OtherFX_RecycleNew(driverAudio, TURBO_AUDIO_SFX_ID = 0xE, ..)` every frame while the turbo (reserves) thread lives; the game nudges its distortion up each frame |
| `hop.wav` | OtherFX 0x08 → SPU 7 | 15999 Hz | 0.17 s | `Vehicle/VehPhysGeneral.c`: `OtherFX_Play_Echo(VEH_PHYS_JUMP_NORMAL_SFX = 8, ..)` |
| `land.wav` | OtherFX 0x07 → SPU 6, volume 0x78 | 15999 Hz | 0.41 s | `COLL.c` ~1513: on first ground contact `OtherFX_Play_LowLevel(7, 1, volume)` with `volume = MapToRange(abs(landing speed), 0x100, 0x3C00, 0x78, 0xFA)`; soft end |
| `land_hard.wav` | same, volume 0xFA | 15999 Hz | 0.41 s | same call, hard end of the range |
| `boost_1.wav` | OtherFX 0x0D → SPU 8, distort 0x94, volume 0x80 | 12286 Hz | 1.02 s | `Vehicle/VehFire.c` `VehFire_Audio(VEH_FIRE_AUDIO_SFX = 0xD)` with `fireLevel = numBoostsSuccess << 6` (`VehPhysProc.c` `PowerSlide_Update`): first boost (< 0x40) |
| `boost_2.wav` | same, no distortion, volume 0xC0 | 11025 Hz | 1.14 s | second boost (< 0x80) |
| `boost_3.wav` | same, distort 0x6C, volume 0xFF | 9893 Hz | 1.27 s | third boost (also voice line 0x10) |
| `meter_full.wav` | OtherFX 0x0F → SPU 87 | 22621 Hz | 0.47 s | `Vehicle/VehPhysProc.c` ~2528: when `turbo_MeterRoomLeft` reaches 0, `OtherFX_Play_Echo(VEH_PHYS_PROC_DRIFT_METER_FULL_FX = 0xF, ..)` |
| `crash.wav` | OtherFX 0x06 → SPU 5 | 11025 Hz | 0.83 s | `COLL.c` ~2633: hard wall impact, `OtherFX_Play_LowLevel(6, 1, volume max)` (+ voice line 6, rumble) |

The three boost files come from one sample. The game's "distortion" is a pitch factor,
`distortConst_OtherFX[d] / 0x10000 = 2^((d - 0x80) / 128)`, baked in as the WAV rate
(0x94 → ×1.114, 0x6C → ×0.897). The event volume is baked in as gain (`volume / 255`).
Instrument volumes (`OtherFX.volume`, e.g. hop 127, turbo 102, crash 153, engine 153) are
**not** baked in. They are listed in `sounds.json` for mixing; the game plays at
`vol_FX * instrument_volume * event_volume >> 10`.

Weapon and pickup sounds are OtherFX ids played at full event volume with no distortion
(`OtherFX_Play` / `PlaySound3D`); each row's decomp site is in `sounds.json`:

| file | OtherFX | rate | length | event |
|---|---|---|---|---|
| `roulette.wav` (loop) | 0x5D | 12371 Hz | 0.51 s | item roulette spinning (`RB_Crate.c:202`) |
| `roulette_stop.wav` | 0x5E | 6944 Hz | 0.64 s | roulette lands (`VehPhysProc.c:21`) |
| `roulette_juiced.wav` | 0x41 | 11025 Hz | 0.92 s | roulette lands with ≥ 10 Wumpa (`VehPhysProc.c:22`) |
| `crate_break.wav` | 0x3C | 9991 Hz | 0.68 s | any crate breaks (`RB_Crate.c:150`) |
| `wumpa.wav` | 0x43 | 15999 Hz | 0.22 s | loose fruit picked up (`RB_Fruit.c:60`) |
| `wumpa_count.wav` | 0x42 | 11025 Hz | 0.17 s | fruit reaches the HUD counter (`UI_RenderFrame.c:235`) |
| `bomb_launch.wav` / `bomb_roll.wav` (loop) / `bomb_explode.wav` | 0x47 / 0x48 / 0x49 | 9991 / 8742 / 8000 Hz | 0.61 / 0.60 / 1.34 s | `VehPickupItem.c:151`, `RB_MovingExplosive.c:50`, `:379` |
| `missile_launch.wav` / `missile_fly.wav` (loop) / `missile_explode.wav` | 0x4A / 0x4B / 0x4C | 11025 / 12995 / 8000 Hz | 1.04 / 0.81 / 1.06 s | `VehPickupItem.c:152`, `RB_MovingExplosive.c:43`, `:392` |
| `missile_lock.wav` | 0x56 | 8000 Hz | 0.15 s | lock-on beep (`UI_RaceHud.c:31`) |
| `mine_drop.wav` | 0x52 | 12995 Hz | 0.46 s | TNT / Nitro / potion dropped (`VehPickupItem.c:153`) |
| `tnt_land.wav` / `tnt_jump.wav` / `tnt_honk.wav` | 0x50 / 0x51 / 0x3E | 11025 / 14998 / 8990 Hz | 0.65 / 0.36 / 0.15 s | TNT on a head, each shake-off hop, countdown tick (`RB_GenericMine.c:284`, `RB_TNT.c:289`, `:208`) |
| `tnt_explode.wav` | 0x3D | 14998 Hz | 2.17 s | TNT and Nitro explode (`RB_TNT.c:32`) |
| `glass.wav` | 0x3F | 11994 Hz | 1.14 s | potion or Nitro shatters (`RB_GenericMine.c:397`) |
| `shield.wav` / `shield_pop.wav` / `shield_roll.wav` (loop) | 0x57 / 0x58 / 0x59 | 11025 Hz | 0.58 / 0.57 / 0.69 s | `VehPickupItem.c:154`, `RB_MaskShieldCloud.c:261`, `RB_MovingExplosive.c:56` |
| `aku.wav` / `uka.wav` | 0x53 / 0x54 | 13997 Hz | 1.38 / 0.64 s | mask pickup, `STATIC_AKUAKU/UKAUKA + 0x1A` (`VehPickupItem.c:284`, `:305`) |
| `clock.wav` | 0x44 | 11025 Hz | 1.55 s | `VehPickupItem.c:155` |
| `warp_launch.wav` / `warp_fly.wav` (loop) / `warp_hit.wav` | 0x4D / 0x4E / 0x4F | 11025 / 12995 / 11025 Hz | 0.80 / 0.83 / 0.40 s | `VehPickupItem.c:156`, `RB_Warpball.c:605`, `:66` |
| `countdown.wav` / `go.wav` | 0x45 / 0x46 | 11025 / 14707 Hz | 0.63 / 0.47 s | start lights: each red, then the green (`DotLights.c:26-27`, `:108`, `:95`) |

Not produced: **`boost_early.wav`**. A too-early boost (`PowerSlide_Update` sets
`failedBoostExhaustTimer`) only changes the exhaust particles in `VehEmitter.c` and plays no
sound. Other sounds exist if wanted:
- racer-vs-racer collisions, `OtherFX_DriverCrashing` (`HOWL_OtherFXRecycle.c`): 0x0A (loud),
  0x0C, 0x0B (soft);
- 0x10, the small bump when the kart bottoms out (`COLL.c` ~1617);
- 0x09, the spring/forced jump.

### Engine pitch

`engine.wav`'s natural rate (12274 Hz) is the game's pitch at engine distortion `d = 0x80`.
`EngineSound_Player` sets
`d = MapToRange(pitchState, 0, accelSpeed + sacredFireSpeed + 0xF00, 0x3C, 200)`
(plus a drift offset while sliding, capped at 0xFF). `pitchState` smooths toward the kart's
speed, or toward `fireSpeed + 0xF00` while airborne or drifting. The sample plays at
`pitch * distortConst_Engine[d] >> 16`, and `distortConst_Engine[d] = 0x10000 * 2^((d - 128) / 64)`.

So **playback-rate factor = 2^((d − 128) / 64)**:

| d | factor | meaning |
|---|---|---|
| 0x3C | ×0.479 | standstill |
| 128 | ×1.0 | about half the speed range |
| 200 | ×2.18 | top speed |
| 0xFF | ×2.69 | maximum |

A linear speed→rate mapping is `factor = 2^((60 + 140 * speed / top_speed − 128) / 64)`.

### Verifying

```sh
cargo run -p ctr-assets --release --example waveform -- kart-data/audio kart-data/audio-waveforms.png
```

This prints duration, peak, RMS and loop-seam jump per file and draws each waveform (loops
are drawn twice with a red tick at the seam). With `CTR_DISC` set, `tests/disc.rs` also
checks every file's length, level and loop flag.

## Effects

`--effects` writes `<out_dir>/effects.json`: CTR's kart visual effects as the game draws
them. Textures and the flame models come from the disc (`shared.mpk`, `shared.vrm`). The
spawn and animation numbers are code constants of the executable, transcribed from the
CTR-native decompilation and cited per effect. Everything is converted to kart.json space:

- meters with +Y up, +Z forward and +X left; 1 CTR world unit = 1/51.2 m (kart model unit
  / 64 = world / (0.8 × 64));
- seconds, where the game updates effects once per frame at 30 fps.

Particle axes are FP8 world units per frame (`>> 8`); the converted values are per second.

### Schema

```jsonc
{
  "format": "ctr-effects", "version": 1,
  "units": { … },
  "textures": [ { "name", "width", "height", "rgba": [ … ] } ],   // RGBA8, rows top-first
  "turbo_flame": {
    "frames": [ { "name": "turbo", "surfaces": [ { "texture": 0 | null,
                   "blend": "additive" | "subtractive" | "mix" | "add_quarter" | "opaque",
                   "vertices": [ { "position": [x,y,z], "uv": [u,v], "color": [r,g,b,a] } ],
                   "indices": [ … ] } ] } ],                         // 8 frames
    "instances": [ { "name": "left", "origin": [x,y,z], "mirror_x": false },
                   { "name": "right", "origin": [x,y,z], "mirror_x": true } ],
    "animation", "placement", "fire_size", "fade", "spawn", "colors", "source"
  },
  "exhaust_smoke": { "texture", "blend", "blend_boost_ready", "boost_ready_rule", "color",
                     "lifetime_seconds", "size", "rotation", "spawn_points", "velocity", "spawn" },
  "failed_boost_puff": { "frames", "seconds", "uses": "exhaust_smoke", "effect" },
  "skid_marks": { "texture", "blend", "color", "width_m", "tires", "laying", "ring_frames", "spawn" },
  "dirt_dust": { "kind": "line", … },
  "landing_sparks": { "kind": "line", … }
}
```

Blend modes are the PSX ones:

| mode | result |
|---|---|
| `mix` | 0.5·dst + 0.5·src |
| `additive` | dst + src |
| `subtractive` | dst − src |
| `add_quarter` | dst + 0.25·src |

On a blended surface, a texel with alpha 128 (STP bit set) blends, alpha 255 draws opaque,
and alpha 0 is transparent. Vertex `color` multiplies the texture, with 1.0 = PSX 0x80.

### Turbo flame

Source: `Vehicle/VehTurbo.c` `VehTurbo_ThTick` and `Vehicle/VehFire.c` `VehFire_Increment`.

- **Geometry:** 8 small 3D models, `turbo`…`turbo7` in `shared.mpk` (ids 0x2C–0x33,
  `STATIC_TURBO_EFFECT`), each ~42 triangles. Each has a bright additive core, an additive
  scrolling-flame shell and a subtractive outer shell.
- **Instances:** a turbo thread holds two flames, left and right. The right flame is the
  left one with X negated.
- **Origins:** `(±0.281, 0.750, −0.812)` m, from `scale·9>>11`, `scale·3>>8` and
  `scale·−0x34>>12` world units with the racer scale 0xCCC. The flame extends up, back and
  outward along the pipes.
- **Animation:** every game frame `fireAnimIndex` advances by 1 (wrapping at 8). The left
  flame shows model `i`, the right flame model `(i+3)&7`. One loop is 8 frames, 0.27 s.
- **Size:** uniform scale `fire_size/8`, with `fire_size = clamp(5 + (fire_level >> 6), 4, 8)`.
  The frames are exported at size 8.
  - Power-slide boosts pass `fire_level = boosts_so_far << 6`, giving sizes 5, 6, 7.
  - Start boost, turbo pads, the turbo item and sacred fire (0x100) give 8.
  - There is no separate blue flame model.
- **Visibility and fade:**
  - The flame is hidden for 96 ms after a non-pad boost.
  - `alphaScale` (0 = visible … 4096 = gone) fades the flame out: fast (0x100/frame) after
    the 2-frame disappear countdown that power-slide flames get, slow (0x40/frame) once
    reserves < 16.
  - The thread dies at 4096.

### Exhaust smoke

Source: `Vehicle/VehEmitter.c` `VehEmitter_Exhaust` and the exhaust block of the driver
emitter routine; `zGlobal_DATA.c` `emSet_Exhaust_High`.

- **Sprite:** the billboard `dustpuff00` (32×32). All five dustpuff frames are exported, but
  the exhaust only uses frame 0.
- **Spawn points:** `anchors.exhaust` in kart.json, `(±0.281, 0.875, −0.875)` m
  (`scale·9>>3`, `scale·7>>1`, `scale·−0x38>>4` in FP8).
- **Rate:** one puff per pipe per frame (60/s in 1P; 2P every other frame, 3P/4P every 4th).
  - Always emitted, except while a turbo flame burns.
  - The exception lifts when the drift meter is in its boost window or
    `failedBoostExhaustTimer > 0`.
  - No puffs at `revEngineState == 2`.
- **Motion:**
  - Initial velocity is kart-local `(0, +2.34, −2.34)` m/s (up and back, `(0, 0x400, −0x400)`
    FP8/frame), with gravity −17.6 m/s².
  - Puffs follow the kart's translation but not its rotation (`DRIVER_LOCAL` particles).
- **Size:** half-extent 0.234 m, growing 1.83 m/s.
- **Lifetime:** 6 frames (0.2 s).
- **Colour:** gray 64/255, falling 10/255 per frame.
- **Rotation:** random screen-plane spin.
- **Blend:** additive normally. **Subtractive (dark smoke) in the boost window**: while the
  drift meter is between 129 and `(const_turboLowRoomWarning + 2)·32`, or at the right rev
  moment on the start line. This is CTR's "boost now" cue.

**Failed boost:** `PowerSlide_Update` sets `failedBoostExhaustTimer = 8` when the player
boosts too early. For those 8 frames (0.27 s) the normal additive exhaust is emitted even
while a flame is burning. There is no dedicated particle.

### Skid marks

Source: `VehEmitter_Skidmarks` (laying) and `Vehicle/VehGroundSkids.c` (drawing).

- **Texture:** global icon 0x2F `tread` (16×4).
- **Look:** drawn subtractive with intensity 0x7F near the camera, fading with distance.
- **Laying:**
  - Each frame each skidding tire writes a cross-section: two ground points 0.39 m apart
    (half-width 10 world units), perpendicular to the heading.
  - Consecutive cross-sections are joined into quads.
  - The buffer is an 8-frame ring, so a mark is a short 0.27 s trail that stays in world
    space.
- **Tires:** rear `(±0.586, 0, −0.39)`, front `(±0.586, 0, 0.78)` m.
- **Rear tires mark when:**
  - power-sliding (`VehPhysProc_PowerSlide_PhysLinear` sets `ACTION_BACK_SKID |
    ACTION_FRONT_SKID`);
  - hard cornering or braking (`VehPhysForce.c`);
  - at wheelie start;
  - on forced-skid terrain.
- **Front tires mark** when power-sliding or on forced-skid terrain.
- **Always:** only with the wheels on the ground.

### Drift dust / sparks

- **`dirt_dust`** (`VehEmitter_Terrain_Ground`, `emSet_DirtLR` / `emSet_Grass*`):
  - Only on terrains with an emitter (dirt, grass, …); asphalt, track and stone have none,
    so CTR shows **no drift dust on roads**.
  - Short additive G2 lines (tail black, head orange (1, 0.5, 0) fading), lifetime 0.33 s,
    launched up at 3.4–4.3 m/s with −20.6 m/s² gravity.
  - One per rear tire per frame, or per all four tires while drifting; requires
    |speed| ≥ 0x300; 1P only.
- **`landing_sparks`** (`VehEmitter_Sparks_Ground`, `emSet_GroundSparks`):
  - 10 additive spark lines (yellow-white → dark, 0.27 s) on the landing frame after a big
    jump (jump height > 22.0 FP8, |speed| > 6.0 FP8).
  - Only on terrain with `TERRAIN_FLAG_LANDING_SPARKS` (asphalt, stone, icy road); 1P only.
- **No drift sparks:** powerslides on asphalt produce no sparks. Wall-scrape sparks
  (`emSet_WallSparks`) exist but are not exported.

### Verifying

```sh
cargo run -p ctr-assets --release --example render_effects -- kart-data/effects.json kart-data/render-effects.png
cargo run -p ctr-assets --release --example render_kart -- kart-data/kart.json kart-data/render-crash-turbo.png --effects kart-data/effects.json
```

The first command renders the 8 flame frames (side and rear, PSX blend modes), a smoke puff
over its life in both blend modes, and every effect texture. The second renders the kart
with both flames attached at fire size 8 and the exhaust anchors (white markers).

## Weapons

`--weapons` writes `<out_dir>/weapons.json`: the weapon, crate and Wumpa fruit models with
every stored animation frame and their textures, plus the pickup timing constants.

- **Weapons** come from `shared.mpk` / `shared.vrm`, looked up by name and checked against
  their `gGT->modelPtr` id.
- **The "?" crate, fruit crate and Wumpa fruit** exist only in the track level files. They
  are read from Crash Cove 1P (`levels\tracks\coco1\1P`, bigfile 112 `data.vrm` /
  113 `data.lev`), with its VRM loaded on top of `shared.vrm`; every track carries the same
  three models. The level header holds `num_models @0x14` and `ptr_models @0x18`.
- **Frames** use the `_hi` mesh (`meshes[0]`) and the effects conversion (`mesh_frame`), so
  units, blend modes, colours and texture alpha are exactly as in [Effects](#schema):
  meters at instance scale 1.0 (0x1000, `INSTANCE.c:31`), +Y up.

### Schema

```jsonc
{
  "format": "ctr-weapons", "version": 1,
  "units": { … },
  "textures": [ { "name", "width", "height", "rgba": [ … ] } ],
  "models": {
    "<key>": {
      "source", "model", "model_id", "mesh",
      "animation_name": "thrust" | null,
      "frames": [ { "name", "surfaces": [ … ] } ],   // as effects.json turbo_flame.frames
      "game_frames": 10,          // = frames.len(), unless stored at half rate (frames.len() = game_frames / 2 + 1)
      "frame_rate_fps": 30, "animation": "…",
      "instance_scale": 1.0,      // the game's spawn scale (0x1000 = 1.0)
      "bounds": { "min": [x,y,z], "max": [x,y,z] }   // over all frames, at scale 1.0
    }
  },
  "constants": { "roulette", "crate_respawn", "fruit_crate_wumpa", "loose_fruit",
                 "wumpa_juiced", "item_choice" }
}
```

| key | model (id) | frames | notes |
|---|---|---|---|
| `crate_question` | level `crate_question` (0x08) | 1 | weapon crate, ~2.1 m cube standing on y 0 |
| `crate_fruit` | level `crate_fruit` (0x07) | 1 | gives 5–8 Wumpa |
| `fruit` | level `fruit` (0x02) | 1 | untextured Gouraud; its 32 `spin` frames are byte-identical on the disc, so one is exported |
| `bomb` | `bomb` (0x3B) | 1 | |
| `missile` | `rocket` (0x29) | 10 `thrust` | body near the origin, nose +Z; the flame and sparks trail to −6.8 m |
| `tnt` / `nitro` | `crate_tnt` (0x27) / `crate_nitro` (0x06) | 1 | Nitro is the juiced TNT |
| `crate_explosion` | `crate_explosion` (0x26) | 12 `break`, once | crate debris |
| `beaker_green` / `beaker_red` | `beakergreen` (0x47) / `beakerred` (0x46) | 15 `bubble` | green is the normal potion, red the juiced one (`VehPickupItem.c:1314/1327`) |
| `cloud` | `cloud` (0x42) | 23 `billow` | potion rain cloud |
| `shockwave_red` / `shockwave_green` | (0x44 / 0x45) | 17 `wave`, once | TNT ring / other blasts |
| `shield_dark`, `shield_green`, `shield_blue`, `shield_highlight` | `shielddark` (0x5A), `shield_green` (0x5E), `shield` (0x56), `highlight` (0x5D) | 1 | scale 0x700; dark base + colour layer (green normal, blue juiced) + highlight |
| `aku` / `uka` | `akumouth` (0x39) / `ukamouth` (0x3A) | 7 `speak` (13 game frames) | |
| `akubeam` / `ukabeam` | (0x3E / 0x40) | 24 `turn` | |
| `warpball` | `warpball` (0x36) | 16 `warp` | |
| `warpedburst` | `warpedburst` (0x2B) | 30 `burst`, once | bomb/missile/orb explosion |

The clock has no model (a screen effect plus `clock.wav`). The HUD's item icons, the
spinning `fruitdisp` and the missile lock-on square (`lock_on` layout element, icon 0x2D) are
in `hud.json`.

### Verifying

```sh
cargo run -p ctr-assets --release --example render_weapons -- kart-data/weapons.json kart-data/render-weapons
```

writes one PNG per model: up to 8 evenly spaced frames from the front-left and the
back-right (orthographic, z-buffered, PSX blend modes).

## HUD

`--hud` writes `<out_dir>/hud.json`: CTR's 1-player race HUD in speedometer mode.

- **From the disc:** sprites (fonts, item icons, racer portraits) from `shared.mpk` icons
  and `shared.vrm`, and two small Gouraud meshes: the big rank digits `big1`…`big8` and the
  Wumpa fruit `fruitdisp`.
- **From the decompilation:** the layout (`zGlobal_DATA.c` `hud_1P_P1`) and the drawing
  rules (`game/UI/*.c`, `DecalFont.c`, `DecalHUD.c`).
- **Coordinates:** CTR framebuffer pixels, **512×216**, +y down. The PS1 shows this on a 4:3
  TV, so pixels are 0.5625 as wide as tall. For square pixels use x·0.5625 (a 288×216
  canvas).
- **Sprite colour:** a sprite's colour multiplies its texture, with 0x80 = 1.0.

### Schema

```jsonc
{
  "format": "ctr-hud", "version": 1,
  "screen": { "width": 512, "height": 216, "pixel_aspect": 0.5625 },
  "textures": [ { "name", "width", "height", "rgba" } ],
  "sprites": [ { "name", "texture", "size": [w, h] } ],     // drawn size at scale 1, top-left anchored
  "fonts": [ { "name": "big" | "small", "advance", "advance_punct", "line_height",
               "glyphs": [ { "ch": "a", "sprite": 12 } ] } ],  // lower-case keys, text is drawn upper-case
  "colors": { "orange": [TL, TR, BL, BR], "periwinkle", "red", "white", "gray_neutral" },
  "meshes": [ { "name": "big1".."big8" | "fruitdisp",
                "positions": [[x, y], …], "colors": [[r, g, b], …],    // triangles, px relative to anchor
                "positions3d": [[x, y, z], …] } ],                     // unprojected (+y up), to spin the fruit
  "elements": { "portraits": { name: sprite }, "rank_icons_3p4p": [ … ],
                "layout": { "rank_list", "weapon", "lap", "wumpa", "rank",
                            "lock_on", "speedometer", "slide_meter", "jump_meter", "start_lights", "mode_note" } }
}
```

### Fonts (`DecalFont.c`)

| font | glyphs | advance | `.` and `:` | line height | source |
|---|---|---|---|---|---|
| `big` | 16×16 | 17 px | 11 | 17 | icon group 4 |
| `small` | 12×8 | 13 px | 7 | 8 | icon group 5 |

- Digits, a–z and `' ! : , = - % . + ? _ /` are mapped (`font_characterIconID`).
- Right justification subtracts the line width.
- Colours are 4-corner gradients:
  - orange: yellow top → red bottom;
  - periwinkle: lavender;
  - white.

### Layout (512×216 px)

| element | where | source |
|---|---|---|
| rank list | rows `i = 0..3`: portrait at (20, 57+27i), place number (small, white; red once that place has finished) at (52, 56+27i); top 4 only | `UI_Rank.c` `UI_DrawRankedDrivers` |
| item | (232, 5), top-left; `item_<name>` sprites (exported upright) | `UI_Weapon.c` |
| item count | small digit at (232, 5) when holding 3 | `UI_Weapon.c` |
| juiced item | with ≥10 wumpa: juiced TNT/potion/shield icons, and a pulsing red `juiced_shine` at (286, 9), scale 0.6 | `UI_Weapon.c` |
| lap | "LAP" small periwinkle, right-aligned at x 484, y 8; "2/3" big periwinkle, right-aligned at x 484, y 16 | `UI_RaceHud.c` `UI_DrawLapCount` |
| wumpa | spinning fruit mesh centred at (316, 24); "x" small orange at (336, 20); count big orange at (349, 16) | `UI_DrawNum.c` `UI_DrawNumWumpa` |
| place | big digit mesh (`big<rank>`) centred at (50, 191); suffix "ST/ND/RD/TH" big orange at (70, 171) | `UI_DrawPosSuffix`, `UI_Instance.c` |
| speedometer | dial at origin (480, 190), needle pivot (479, 186) | `UI_Speedometer.c` |
| drift meter | box x 441–490, y 199–206 | `UI_Meter.c` `UI_DrawSlideMeter` |
| jump meter | bar x 491–503, y 168–206; readout box x 471–505, y 161–171 | `UI_Meter.c` |
| start lights | four 47×31 lights in a row, x 162–350, bottom edge at y 72 in place (`start_lights` element: `lightredoff`, `lightredon`, `lightgreenoff`, `lightgreenon`) | `DotLights.c` `DotLights_Video` |
| race clock | "TIME" (or "TIME TRIAL") small orange at (20, 8); "m:ss:cc" big orange at (20, 16); lap row n (from 1) at y 8 + 8n + 16: "Ln" small red at x 20, " m:ss:cc" small periwinkle at x 46 | `UI_Clock.c` `UI_DrawRaceClock`, `UI_RenderFrame.c:561` |
| "FINAL LAP" | big orange, centred on the screen, y = h/4; 90 frames: in from w + 100 to w/2 over 10, held to 80, out to −100 over 10 | `MainFrame_RenderFrame.c` `DrawFinalLap` |
| "WRONG WAY" | big orange, centred, y = h/2 − 0x1E; lit while bit 0x80 of the race time (1/960 s) is set | `UI_RenderFrame.c:130-160` |

The rank and fruit meshes are flattened from the game's HUD instances:
`screen = model · scale/4096 · 256/z`, with y × 0.5625 and flipped to +y down. Rank is
scale 5120 at z 256; the fruit is scale 4096 at z 512.

### Drawing rules

- **Speedometer** (`UI_DrawSpeedBG`, `UI_DrawSpeedNeedle`)
  - *Dial:*
    - Seven spokes.
    - Six Gouraud quads coloured green, green, green→yellow, yellow→red, red, red.
    - 1-px white outline lines plus a black copy offset by (+1, +1).
    - 50% black semi-transparent backdrop.
  - *Needle:* two Gouraud triangles (dark/bright gold), length 60 px, y × 0.625.
  - *Needle angle* (CTR convention: x = sin, y = cos, y down):
    - 305.2° → 213.75° as speed goes 0 → `accel_speed`;
    - 213.75° → 157.5° from `accel_speed` → `accel_speed + sacred_fire_speed`.
    - So the needle sweeps from lower-left, through roughly upper-left at normal top speed,
      to upper-right at full boost.
- **Drift meter** (`UI_DrawSlideMeter`)
  - Gray box 49×7 with a black outline.
  - While power-sliding, `turbo_MeterRoomLeft` counts down from `const_turboMaxRoom`. The
    fill `49 − 49·room/max` grows leftwards from x = 490.
  - The fill is **green** until `room <= const_turboLowRoomWarning`, then **red**: the boost
    window (the exhaust also goes dark then; see effects).
- **Jump meter** (`UI_JumpMeter_Update/Draw`; speedometer mode only)
  - Airtime in 1/960 s, clamped to 2400.
  - Fill height = 38·t/2400 from the bottom.
  - Colours mark the hang-time boost tiers: **red** < 0.67 s, **green** < 1.0 s, **cyan**
    < 1.5 s, **blue** ≥ 1.5 s.
  - Held for 1.5 s after landing, then cleared.
  - The white readout box shows S.T H in the debug font, which is not exported.
- **Item flicker:** every other frame while `noItemTimer` runs.
- **Start lights** (`DotLights_AudioAndVideo`, on `trafficLightsTimer`, 960 units a second,
  0xF00 as the race starts)
  - 0xF00 → 0xB40: all off, sliding down from above the screen, `y = 72·(0xF00 − t)/960 − 31`.
  - ≤ 0xB40, ≤ 0x780, ≤ 0x3C0: the first, second and third red on, each with OtherFX 0x45.
  - ≤ 0 (GO): the green on with OtherFX 0x46, then sliding back up, `y = 72·(t + 0x3C0)/960 − 31`,
    gone below −0x3BF.
  - A sound plays when the timer crosses into its stretch (`trafficLightsTimer_prevFrame`).

Not exported:
- the track-map mode;
- 2P–4P layouts (their tables are in `zGlobal_DATA.c`);
- the debug font for the jump readout;
- the item roulette's fly-in lerp;
- the 3D C-T-R letters and tokens.

### Drawing it in-game (`hud_draw.rs`)

`ctr_assets::hud_draw` turns `hud.json` into a per-frame overlay.

```rust
pub const WIDTH: usize = 512;
pub const HEIGHT: usize = 216;
pub const CROP_LEFT: [i32; 2] = [0, 160];   // the race clock ends at 150
pub const CROP_MIDDLE: [i32; 2] = [160, 400]; // the start lights begin at 162
pub const CROP_RIGHT: [i32; 2] = [400, 512];

impl HudRenderer {
    pub fn load(path: &Path) -> Result<Self>;          // parses once, pre-decodes sprites
    pub fn from_json(j: &serde_json::Value) -> Result<Self>;
    pub fn render(&self, state: &HudState, out: &mut [u8]); // 512*216*4 RGBA8, rows top-first
    pub fn render_banners(&self, b: &Banners, width: usize, out: &mut [u8]); // width*216*4
    pub fn regions(&self) -> Vec<Region>;              // { name, rect: [x, y, w, h], anchor }
}
pub enum Anchor { TopLeft, Top, TopRight, BottomLeft, BottomRight }
pub struct RaceClock { total: f32, time_trial: bool, laps: Vec<f32> } // HudState::clock
pub struct Banners { final_lap_frame: Option<u32>, wrong_way: bool }
```

The race clock is part of `HudState` and draws in the left crop. "FINAL LAP" and "WRONG WAY"
centre on the whole view, so `render_banners` draws them onto a second canvas as wide as the
screen in CTR's pixels (512 at 4:3, 683 at 16:9); a client shows it over the whole window only
while one of them is up.

`render` clears the buffer to fully transparent and composites with straight-alpha "over":

- HUD pixels are opaque.
- The speedometer backdrop stays at alpha ≈128.
- Nothing is allocated per frame (the fruit's depth sort uses a fixed stack array).
- It takes ~0.16 ms per frame for a full state in release.

`HudState` (`Default` draws nothing):

| field | meaning |
|---|---|
| `speedometer: bool` | dial and needle |
| `speed`, `accel_speed`, `sacred_fire_speed: f32` | CTR speed units; only ratios are used |
| `drift_meter: Option<(room_left, max_room, low_warning)>` | any consistent time unit. `None` draws the empty gray meter (the game shows it all race) |
| `hide_drift_meter: bool` | hides the drift meter entirely |
| `jump: Option<u32>` | airtime in milliseconds. `None` hides the meter. The caller does the 1.5 s hold after landing |
| `wumpa: Option<u8>`, `juiced: bool` | wumpa counter and fruit; juiced item icons and shine |
| `lap: Option<(u8, u8)>` | current and total laps |
| `place: Option<u8>` | 1..=8 |
| `item: Option<(heldItemID, count)>` | CTR item ids; 0x10 = roulette |
| `ranks: Vec<String>` | top-4 portrait names |
| `time_seconds: f32` | fruit spin (0.9375 turns/s), shine pulse, roulette |
| `start_timer: Option<i32>` | the start lights at CTR's traffic-light timer (`lights_timer(seconds to GO)`); `start_lights` and `start_light_sound` give the lit lights, the slide and each light's sound |

For a 16:9 screen, draw the canvas in three horizontal crops:

- `CROP_LEFT` (x 0–120) pinned to the left edge: rank list, place.
- `CROP_MIDDLE` (x 120–400) pinned to the centre: item, wumpa, start lights.
- `CROP_RIGHT` (x 400–512) pinned to the right edge: lap, speedometer, drift and jump meters.

`regions()` gives every element's rectangle; each lies entirely inside one crop.
`examples/render_hud.rs` is a thin wrapper over the renderer and also writes the raw RGBA
overlay; a third argument draws the start lights at that timer.

## Music

CTR's **race music is not CD-XA**. The XA files on the disc (`XA\MUSIC\S00.XA`, `S01.XA`,
`XA\ENG\…`) hold only race start/win/lose stings, jingles and voice lines. In-race music is
sequenced on the SPU from **CSEQ** files inside `SOUNDS\KART.HWL`, with instruments from the
HOWL sample banks. `--music` renders those songs to stereo WAV with a small SPU model and
writes `<out_dir>/music/<track>.wav` plus `music/music.json`.

- **CSEQ layout** (`include/namespace_Howl.h`, `HOWL_Load.c` `howl_ParseCseqHeader`):
  - header `{u32 size; u8 n_long; u8 n_short; u16 n_songs}`;
  - long instruments `{u8; u8 vol; s16; u16 base_pitch; s16 spu_index; u16 ad; u16 sr}`;
  - drums `{u8; u8 vol; s16 pitch; s16 spu_index; s16}`;
  - `s16 song_offset[]`, then aligned song data.
  - Song: `{u8; u8 n_seqs; s16 bpm; s16 tpqn; u16 seq_offset[]}`.
  - Each sequence: `{u8 flags (1 = drums); u8}` then events: MIDI-style variable-length
    delay, opcode, args (`opcodeOffset = {1,2,2,1,2,3,2,2,2,2,2}`).
  - Opcodes:

    | opcode | meaning |
    |---|---|
    | 1 | note off |
    | 3 | end (loop) |
    | 5 | note on (note, velocity) |
    | 6 | sequence volume |
    | 7 | pan |
    | 8 | reverb |
    | 9 | instrument |
    | 0xA | pitch bend |
- **Which song** (`HOWL_Music.c`, `zGlobal_DATA.c`): level `L` loads HOWL sequence
  `data.levBank_Song[L]` (= L for the 18 race tracks) and plays song 0 (`CSEQ_SONG_LEVEL`),
  looping. Songs 1 and 2 are the Aku/Uka mask themes.
- **Timing** (`SongPool_CalculateTempo`, `Channel_ParseSongToChannels`): `bpm·tpqn/60`
  ticks per second, processed in 60 Hz frames. Each sequence restarts at its own end.
- **Voices** (`HOWL_CseqOpcode.c`):
  - *Instrument pitch:* `base_pitch · 2^((note − 60 + (bend − 0x80)/64)/12)`
    (`howl_InstrumentPitch`, `noteFrequency`, `distortConst_Music`).
  - *Drum pitch:* `pitch · 2^((bend − 0x80)/128)`, with fixed ADSR 0x80FF/0x1FC2.
  - *Volume:* `vol_Music(175) · song(0xBE) · seq_vol >> 10 · inst_vol · velocity >> 15`,
    clamped to 0x3FFF.
  - *Pan:* through the `volumeLR` equal-power table.
  - *Note off:* releases the ADSR.
- **SPU model:**
  - Samples play at `pitch/4096 · 44100` Hz, with linear interpolation and sample loop
    points.
  - PSX ADSR (attack/decay/sustain/release rates, exponential modes) at 44.1 kHz.
  - Mixed in float and peak-normalised per track to −1 dBFS.
  - **No reverb** (the game's SPU reverb is not modelled).
  - Output 22050 Hz stereo 16-bit, chosen to keep the set at 118 MB.
- **Loops:** each file is the song's *second* pass, so notes still ringing from the previous
  pass are included. The file loops end-to-start, and a WAV `smpl` chunk marks it.
  `music.json` has `loop_start_seconds = 0` and `loop_end_seconds = duration`.
- **Final lap:** `HOWL_AudioState.c` raises the tempo by 20 BPM (`Audio_SetMaskSong(20)`).
  `final_lap_bpm_delta` is in `music.json`; a client can approximate it with playback rate
  `(bpm+20)/bpm`. The game first plays the final-lap XA sting (`XA\MUSIC`, music index 6)
  with the song ducked, and raises the tempo once the sting has played 0xE1 of its
  256-sample blocks at 44.1 kHz (about 1.31 s). The sting is not exported: it needs CD-XA
  Mode 2 Form 2 sectors read raw, the `.XNF` index that places index 6, and an XA-ADPCM
  decoder, none of which the extractor has.
- **Shared music:** Slide Coliseum and Turbo Track use byte-identical sequences to Tiny
  Arena; their `music.json` entries point to `tiny_arena.wav` (`same_music_as`).

| track | duration | BPM |
|---|---|---|
| dingo_canyon | 90.8 s | 148 |
| dragon_mines | 96.4 s | 229 |
| blizzard_bluff | 87.3 s | 154 |
| crash_cove | 90.9 s | 190 |
| tiger_temple | 67.7 s | 170 |
| papu_pyramid | 90.3 s | 170 |
| roo_tubes | 89.7 s | 182 |
| hot_air_skyway | 128.0 s | 180 |
| sewer_speedway | 89.3 s | 172 |
| mystery_caves | 74.6 s | 180 |
| cortex_castle | 89.7 s | 182 |
| n_gin_labs | 93.1 s | 165 |
| polar_pass | 82.3 s | 175 |
| oxide_station | 69.3 s | 180 |
| coco_park | 90.6 s | 180 |
| tiny_arena (+ slide_coliseum, turbo_track) | 67.4 s | 185 |

**Verification:**
- Every duration is a whole number of bars at the song's tempo (sequence lengths in ticks
  are multiples of 4·120).
- The onset autocorrelation of the renders peaks at the bar/beat period, e.g. Crash Cove
  1.27 s = 4 beats at 190 BPM.
- `music-waveforms.png` plots all tracks.

## Limitations

- One pose: the kart is exported at `turn[10]`. The steering wheel turns in the game's
  `turn` animation; it is exported static (straight ahead).
- The steering wheel's position comes from the chosen racer's model (it differs slightly per
  driver). Pen's kart has no steering-wheel ring; its anchor falls back to the median of the
  other racers.
- N. Gin, Dingodile, Papu Papu and N. Tropy karts are not offered (hood not separable), nor
  Oxide's hovercraft.
- Wheels are sprites in the game; `kart.json` provides sprites, centres and radius, not a 3D
  wheel mesh.
- Only the NTSC-U bigfile indices are known; PAL/JPN discs would need their own index tables.
