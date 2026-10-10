# CTR in-race chase camera: port spec

Source: decompiled CTR NTSC-U, a checkout of [CTR-native](https://github.com/CTR-tools/ctr-native). Citations are `file:line` relative to that repo
(`game/CAM.c` unless another file is named). Units follow `ctr-physics-spec.md`:

- Positions: the game stores `posCurr` in 1/256 lu, but the camera works in **whole lu**. It uses `posCurr >> 8` everywhere, and `pushBuffer.pos` is s16 lu.
- Angles: 4096 per turn.
- Time: one camera tick per frame, 30 fps, `elapsedTimeMS` = 32.
- Axes: Y up. Forward for yaw `a` is `(sin a, 0, cos a)`, and a positive yaw change turns the kart left (counter-clockwise seen from above). Right = `(-cos a, 0, sin a)`.

Below, `sin`/`cos` take 4096-unit angles. `K` is the kart origin (`posCurr >> 8`, bottom centre of the kart).

---

## 0. Where the code lives

| What | Where |
|---|---|
| Camera thread tick, mode dispatch | `CAM_ThTick` `CAM.c:1770-2349`. Mode 0 (normal race) calls `CAM_FollowDriver_Normal` at `CAM.c:2295`. |
| **Normal chase camera** | `CAM_FollowDriver_Normal` `CAM.c:1047-1746` (the race-relevant part is `1068-1545`). |
| Zoom tables | `data.NearCam4x3 / NearCam8x3 / FarCam4x3 / FarCam8x3` `zGlobal_DATA.c:983-1017`. `struct ZoomData` is in `include/namespace_Camera.h:99-120`. |
| Table selection, L2 near/far toggle | `CAM.c:1814-1844` |
| CameraDC struct | `include/namespace_Camera.h:177-355` |
| Rotation-to-matrix convention | `ConvertRotToMatrix` `game/MATH/MATH_7_MatrixStubs.c:322-339`: `M = Ry(rot.y) * Rx(rot.x) * Rz(rot.z)` |
| Projection setup | `PushBuffer_Init` `game/PushBuffer.c:4-41`, `PushBuffer_SetPsyqGeom` `:146-151`, `PushBuffer_SetMatrixVP` `:276-420` |
| Drift camera spin `rotCurr.w` | `VehPhysProc_PowerSlide_PhysAngular` `game/Vehicle/VehPhysProc.c:1953-2007`. Decay to 0 in normal driving: `VehPhysGeneral_PhysAngular` `game/Vehicle/VehPhysGeneral.c:86-167`. |
| Kart roll `rotCurr.z` (used for camera bank) | `game/COLL.c:1580-1585` |
| Turbo zoom trigger | `VehFire_Increment` `game/Vehicle/VehFire.c:377-381` sets camera flag `0x80` |

The rotation matrices, with `c = cos`, `s = sin` (`MATH_7_MatrixStubs.c:240-272, 322-339`):
```
Ry(t) = [[c,0,s],[0,1,0],[-s,0,c]]
Rx(t) = [[1,0,0],[0,c,-s],[0,s,c]]
Rz(t) = [[c,-s,0],[s,c,0],[0,0,1]]
```
`gte_rtv0` computes `M * v >> 12`. So `Ry(yaw) * (0,0,d)` is `d * (sin yaw, 0, cos yaw)`.

---

## 1. Constants (1 player)

`ZoomData` (`zGlobal_DATA.c:983-1008`). The normal camera uses `NearCam4x3`. Pressing L2 toggles `nearOrFar` 0↔1 (`CAM.c:1814-1836`). The table index is `&NearCam4x3[nearOrFar*2]` (`CAM.c:1838-1844`), so "far" is `FarCam4x3`. `2P` uses the `8x3` tables instead.

| Field | Near (default) | Far (L2) | Meaning |
|---|---|---|---|
| `distMin` | 0xB3 = **179** | 0xE0 = 224 | boom length at low speed |
| `distMax` | 0xE0 = **224** | 0x120 = 288 | boom length at high speed |
| `speedMin` | 0x400 = 1024 (4 lu/frame) | same | speed mapped to `distMin` |
| `speedMax` | 0x1400 = 5120 (20 lu/frame, 600 lu/s) | same | speed mapped to `distMax` |
| `percentage1` | 0xF0 = 240 | same | speed-filter weight (see 2.2) |
| `percentage2` | 0xFF = 255 | same | speed-filter weight when slowing |
| `angle[0]` | 0x80 = 128 | same | screen roll = `rollSmoothed * 128/256` |
| `angle[1]` | 0xE0 = 224 | same | roll filter keep-weight (alpha = 32/256 = 1/8) |
| `angle[2]` | 0x70 = **112** | 0x60 = 96 | look-at height above the kart origin |
| `vertDistance` | 0x40 = **64** | 0x60 = 96 | extra eye height. Also the camera's minimum clearance above ground. |

Other constants in `CAM_FollowDriver_Normal`:
- **Boom pitch**: `rot.x = 0xFF9C = -100` (−8.79°) in 1P/3P/4P, and 0 in 2P (`CAM.c:1142-1148`, `1233-1240`). `sin(100) = 0.15280` and `cos(100) = 0.98826`.
- **Side-offset vector** `(0, 0x40, 0)`, i.e. 64 lu (`CAM.c:1203-1209`).
- **Look-ahead** `damagePitchOffset`: −32 lu when driving normally, 0 while `ACTION_WARP` is set (spin-out, tumble, warp). It moves 8 per frame between the two (`CAM.c:1251-1271`).
- **Turbo push-back**: +2048/256 = 8 lu per frame while the timer runs, capped at 0x6000/256 = **96 lu**. It decays at 512/256 = 2 lu per frame (`CAM.c:1159-1196`).
- Height re-blend: 8 frames (`CAM.c:1401, 1408, 1428-1436`).

---

## 2. Normal 1-player follow mode (`CAM_FollowDriver_Normal`)

### 2.1 Which heading does it follow?

**The eye follows the physics heading `angle` plus the drift spin `rotCurr.w`. It does not use the drawn yaw `rotCurr.y` or the velocity direction `axisRotationX`.**

```
eyeYaw  = rotCurr.w + angle                  + 0x800 + rev     (CAM.c:1152)
lookYaw = rotCurr.w + angle + turnAngleCurr  + 0x800 + rev     (CAM.c:1246)
rev     = 0x800 while R2 is held (look back), else 0           (CAM.c:1073-1098)
```
The look-at point uses `angle + turnAngleCurr`, which is the drawn yaw `rotCurr.y` without `turnWobbleAngle`. The eye does not.

**Yaw has no smoothing of its own.** The boom is rigidly locked to `angle + rotCurr.w` every frame. The only lag comes from the physics:
- `angle` itself follows the steering and velocity-heading convergence.
- `rotCurr.w` is rate-limited (section 3.1).
- `turnAngleCurr` lags inside the physics.

Driving in reverse (stick back) has no special case. The camera stays behind the kart's nose.

### 2.2 Speed-dependent boom length (the one positional filter)

`cameraMoveSpeed` (`cms`) is a filtered `|speedApprox|` (`CAM.c:1100-1140`):
```
x = |speedApprox|
if DIRECTION_CHANGED (first frame, R2 toggled, respawn): cms = x
elif x <  cms:  cms = (240*cms + 1*x)  >> 8     // slowing: decays about 6.25 %/frame
else:           cms = (240*cms + 16*x) >> 8     // speeding up: EMA, alpha 1/16 (tau ≈ 16 frames ≈ 0.53 s)
D = MapToRange(cms, 1024, 5120, distMin, distMax)    // linear, clamped (CAM.c:1166, VehCalc.c:36-56)
D += turboOffset >> 8                                // section 3.3 (CAM.c:1196)
```
The source comment says "90%/99%", but the code really computes `(p1*cms + (256-p2)*x)>>8` with p1 = 240 and p2 = 255 when slowing. When slowing, the result drops fast (about 6 % per frame) until it falls below `x`. Then the speeding-up EMA pulls it back to `x`, so it tracks a falling speed closely. The integer truncation stalls the rise up to 15 units short of `x`, which is negligible.

**Practical consequence:** `speedMax` is 20 lu/frame (600 lu/s), and racing speeds are about 1200-1600 lu/s. So **D sits at 224 (Near) for almost the whole race**. It shrinks toward 179 only below 600 lu/s (start, after crashes, slow terrain). The FOV never changes (section 4).

### 2.3 Eye position

Two GTE transforms use `M1 = Ry(eyeYaw) * Rx(-100) * Rz(-2*rollS_prev)` (`CAM.c:1148-1157`, `1198-1209`):
- `boom = M1 * (0, 0, D)`. Rz does nothing to a pure-Z vector, so `boom = D * (0.98826*sin(eyeYaw), 0.15280, 0.98826*cos(eyeYaw))`.
- `side = M1 * (0, 64, 0)`. Only the x and z components are used.

`rollS_prev` is last frame's smoothed roll (`cDC->desiredRot.x`, updated later at `CAM.c:1288`).

Then (`CAM.c:1211-1231`):
```
base.xz = K.xz + side.xz          ; base.y = K.y
eye.x   = base.x + boom.x
eye.z   = base.z + boom.z
eye.y   = K.y + boom.y + vertDistance           // = K.y + 0.1528*D + 64
```
`eyeYaw = angle + w + 0x800`, so `(sin eyeYaw, cos eyeYaw) = -forward(angle + w)`. Writing it out:
```
f  = forward(angle + w + rev) = (sin, 0, cos)
r  = right(angle + w + rev)   = (-cos, 0, sin)
side = 64*sin(2*rollS)*r + 9.779*cos(2*rollS)*f       // 9.779 = 64*sin(100)
eye  = K - 0.98826*D*f + side + (0, 0.15280*D + 64, 0)
```
When the kart is flat (`rollS = 0`) and Near is at full speed (D = 224):
- **211.6 lu behind** the kart origin (221.4 − 9.8)
- **98.2 lu above** it

At D = 179 the eye is 167 lu behind and 91.4 lu above.

**Mask-grab** (`CAMERA_FLAG_MASK_GRAB`) replaces the eye height with `quadBlockHeight>>8 + maskGrabHeightOffset + vertDistance` (`CAM.c:1221-1224`). Respawn only, so it is not needed.

### 2.4 Look-at point

`M2 = Ry(lookYaw) * Rx(-100)` (`CAM.c:1239-1249`). The look-at is `base + M2*(0, 0, dpo)`, plus `angle[2]` on Y (`CAM.c:1273-1286`):
```
g      = forward(angle + turnAngleCurr + w + rev)
lookAt = K + side + 0.98826*|dpo|*g + (0, 112 - 0.1528*|dpo|, 0)     // dpo = damagePitchOffset (-32 normally)
       = K + side + 31.6*g + (0, 107.1, 0)                            // normal driving
       = K + side + (0, 112, 0)                                       // during spin-out/tumble/warp (dpo -> 0)
```
So the camera aims at a point **31.6 lu ahead of the kart along its drawn nose direction, 107 lu above the origin**. The look-at sits higher than the eye (98 lu), so **the view pitches about 2° up** at D = 224 (about 4.5° at D = 179). The kart is framed low: its origin lands about 27° below the screen centre, roughly 84 % of the way down a 4:3 screen.

The final camera angles come from `dir = eye − lookAt` (`CAM.c:1476-1495`):
```
pb.rot.y = ratan2(dir.x, dir.z)
pb.rot.x = 0x800 - ratan2(dir.y, sqrt(dir.x²+dir.z²))          // 0x800 = level (the PSX Y-down flip)
pb.rot.z = (angle[0] * rollS) >> 8 = rollS / 2                 // screen roll
```
In a port, just use `look_at(eye, lookAt, up)` and then apply the roll about the view axis.

### 2.5 Roll / banking on slopes

**Yes, the camera banks, by half the smoothed kart roll.**
- `rotCurr.z = ratan2(n·right, n.y)` (`COLL.c:1580-1585`), where `n` is the kart's visual ground normal `AxisAngle2_normalVec`. It is the lateral tilt of the surface under the kart, positive when the normal leans to the kart's right.
- The smoothing is `rollS = (224*rollS + 32*rotCurr.z) >> 8` per frame (EMA, alpha 1/8; `CAM.c:1288`).
- The screen roll is `rollS/2` (`CAM.c:1494`).

The same filtered roll also slides the whole rig sideways: `side` contains `64*sin(2*rollS)` along the kart's right. For a 10° bank that is about +22 lu. The sign follows the code as written, using last frame's `rollS`.

Pitch on slopes: the camera pitch is **not** taken from the kart's pitch. Eye and look-at are both offset from `K` with world-up heights, so on a hill the view stays level-ish and only translates with the kart.

### 2.6 Final position write and the ±2 lu "correction"

`pushBuffer.pos = eye + corr`, where `corr = clamp(corr - (eye - eyePrev), -2, +2)` per axis (`CAM.c:1497-1538`). This is a tiny lag of up to 2 lu that hides s16 jitter. The camera angles are computed from the uncorrected eye. **This can be ignored in a float port.**

---

## 3. Special situations

### 3.1 Powerslides: `rotCurr.w` swing

`rotCurr.w` adds directly to the eye yaw and the look yaw (section 2.1). It is computed in physics:

- **While drifting** (`VehPhysProc.c:1953-2007`):
  ```
  target = multDrift >= 0 ? +384 : -384          // DriftCameraSpinRate = 384 (33.75°), signed by drift direction
  stepCap = clamp(|w - target| >> 3, 1, 16)        // 16 = DriftCameraLerpStep
  wVel  = moveToward(wVel, stepCap, 8)            // rotPrev.w, rate-limited "velocity", always >= 0
  w     = moveToward(w, target, wVel)             // per frame (wVel*ms>>5)
  ```
  The swing reaches at most 16 units per frame, so the full 384 takes at least 24 frames (about 0.8 s).
- **When not drifting** (`VehPhysGeneral.c:86-167`): the same form with target 0, `stepCap = clamp(|w|>>3, 1, 16)`. So the camera eases back at up to 16 units per frame.
- In spin-out, wall-slam and spin-stop states it goes to 0 at 32 units per frame (`VehPhysProc.c:2657, 2845, 3009, 3063`).

**Effect, for a left drift** (`multDrift >= 0`, angle increasing):
- The boom rotates +33.75° counter-clockwise (seen from above) around the kart. The eye swings out toward the **outside** of the turn (the kart's right-rear), so you see the kart's outer flank.
- The look-at goes 31.6 lu along `angle + w + turnAngleCurr`. During a drift `turnAngleCurr` is about +768..+1068, so this is about 100° to the left of `angle`. The aim point shifts toward the inside of the turn.

A right drift mirrors this. Wall slam (`COLL.c:2638-2645`) folds `turnAngleCurr` into `angle` and subtracts it from `w`.

### 3.2 Air / jumps

There is no special airborne logic.
- The eye and look-at heights are **rigid offsets from `K.y`**, so the camera rises and falls 1:1 with the kart.
- The only height filters are the ground clamp and its 8-frame re-blend (section 5).
- In the air the roll filter keeps easing toward whatever `rotCurr.z` gives for the airborne normal.

**Weapon blast** (`KS_BLASTED`) is a special case (`CAM.c:1312-1392, 1447-1450`):
- On the first blasted frame it stores the eye's height above the ground (`cameraPos.y − quadBlockHeight`) and the look-at−eye Y delta.
- While blasted, the eye is not allowed to rise above `ground + storedHeight`, and the look-at Y is fixed relative to the eye. So the camera does not follow the flip arc upward.
- When the kart touches ground again it stores `(oldPos − newTarget)` and `(oldLookAt − newLookAt)`. It adds these back scaled by `n/8` for n = 8..1, giving an 8-frame ease-in.

### 3.3 Turbo / speed

- **The FOV never changes with speed or turbo.** `distanceToScreen` is only modified in end-of-race mode 0xB (`CAM.c:2282`).
- **Boom length grows with speed** (179 → 224) but saturates at 600 lu/s (section 2.2).
- **Turbo push-back** (`CAM.c:1159-1196`):
  - Every `VehFire_Increment` on a human kart sets camera flag 0x80 (`VehFire.c:377-381`). This covers drift boosts, hang-time and start boosts, turbo pads, the Turbo item and super engine.
  - On a tick where the flag is set: `timer = max(timer, fireSpeedCap >> 4)` (CTR-ms). For drift boosts `fireSpeedCap` is 2048/2560/3072, giving 128/160/192 ms, which is **4/5/6 frames**.
  - While `timer > 0`: `off = min(off + 2048, 0x6000)`, then `timer -= 32`. So **+8 lu of boom per frame, up to 96 lu**.
  - When `timer == 0`: `off = max(off - 512, 0)`, i.e. **2 lu per frame** back.
  - Boom `D += off >> 8`. Because the boom is pitched, the eye also rises `0.153*Δ`.
  - Resulting push-back: a level-1 drift boost pushes about 32 lu then recovers over about 16 frames (0.5 s). A turbo pad refreshes the timer every frame you are on it (the "frozen reserves" path), so it saturates at +96 lu. A boost while blue fire is active (`fireSpeedCap` up to 6144, 12 frames) also reaches 96.
  - **Uncertainty:** `fireSpeedCap` is read at camera time. If the camera thread runs before `VehFire_Increment` in a frame, the flag is consumed the next frame, a 1-frame delay. Treat this as negligible.

### 3.4 Reverse look (R2) and reversing

- **R2 held** (human, not during intro) adds 0x800 to both yaws. The change sets `DIRECTION_CHANGED`, which resets `cms = |speed|` (`CAM.c:1071-1098, 1129-1140`). No blend: an instant cut.
- **Driving backwards** is not special. The camera stays behind the nose, and `|speedApprox|` still lengthens the boom.

### 3.5 Start-of-race fly-in, end-of-race, battle

These are scripted. The fly-in path is in `CAM.c:1588-1643`, blended by `transitionBlend` with sin/cos easing over `transitionFrameCount` (`CAM.c:1655-1683`). End-of-race uses modes 3-14 from level data (`CAM.c:1846-2032`). They are out of scope for the chase camera.

---

## 4. Projection (FOV)

1P setup (`PushBuffer.c:28-40, 146-151, 381-404`):
- Viewport `rect = 512 × 216`, projection centre `(256, 108)` (`gte_SetGeomOffset(w/2, h/2)`), so there is no off-centre shift.
- **H (`distanceToScreen`) = 256.** It is 128 in 3P/4P (quarter screens 253 wide).
- View Y is scaled by `0x360/0x600 = 9/16` (`PushBuffer.c:389-404`). The code comment: "9/16 * 512/216 = 4/3".

Projection:
```
sx = 256 * X/Z              (pixels, ±256 at the edge)
sy = 144 * Y/Z              (256 * 9/16; ±108 at the edge)
tan(hFOV/2) = 256/256 = 1.0      -> hFOV = 90.00°
tan(vFOV/2) = 108/144 = 0.75     -> vFOV = 73.74°
```
`tan` ratio 1/0.75 = 4/3, so the designed frustum is a **4:3 frustum, 90° horizontal by 73.74° vertical**.

On real hardware the 216 lines are letterboxed into 240 (`MainMain.c:522-535`, `screen.y = 12`). The physical picture is about 1.48:1 and slightly vertically squashed. The angular coverage is still 90° × 73.74°.

For a 16:9 window:

| Choice | vFOV | hFOV | Notes |
|---|---|---|---|
| Original 4:3 | 73.74° | 90.00° | |
| **Hor+ (keep vertical), recommended** | **73.74°** | 106.26° (`2*atan(0.75*16/9)`) | Kart sits at the same height on screen, with more side view. |
| Keep horizontal | 58.72° (`2*atan(9/16)`) | 90.00° | Crops top and bottom. The kart moves toward the screen edge, since its origin is about 27° below centre and the half-angle is only 29.4°. Not recommended. |

The PSX near clip is not a standard frustum plane, so pick your own (e.g. 8-16 lu).

---

## 5. Ground / wall collision

**There is no wall collision and no line-of-sight test.** The camera can clip through walls or behind geometry. The only collision is a **ground-clearance clamp** under the eye (`CAM.c:1394-1445`):

1. **Probe** with `CAM_FindClosestQuadblock` (`CAM.c:584-655`). It sends a vertical segment from `eye.y + 256` down to `eye.y − 2048` against quads flagged `QUADBLOCK_FLAG_CAMERA_SEARCH` (0x0800). It tries the cached quad first, then the BSP. The hit height is `terrainHeightFloor` (scratch `hitPos.y`).
2. **If a hit is found and the quad does not have flags `0x4100`:**
   - If the terrain is mud, water or fast water (types 0xE, 4, 0xD), the floor is 0.
   - `eye.y = max(eye.y, floor + vertDistance)`, i.e. at least 64 lu above ground.
   - If `blendN > 0`: `eye.y = ((8−n)*eye.y + n*(K.y + startOff)) >> 3`, then n--.
3. **If no hit, or the quad has flags `0x4100`:**
   - The eye may not drop below the previous offset: if `eye.y < K.y + curOff`, set `eye.y = K.y + curOff` and `startOff = curOff`.
   - Otherwise `startOff = eye.y − K.y`.
   - Either way, arm the 8-frame blend (`n = 8`).
4. Then `curOff = eye.y − K.y`. This update is skipped in the hold case.
5. Levels with `configFlags & 2` (underwater) clamp `eye.y >= vertDistance` (`CAM.c:1441-1445`).

In practice, with a flat or uphill track the eye is already about 98 lu above `K` and the clamp rarely fires. It matters cresting hills and dropping off ledges. When you land after going off an edge where camera-search quads were missing, the clamp eases in over 8 frames.

Port suggestion: raycast down from the eye against your ground mesh, clamp to `ground + 64`, and keep the 8-frame blend. Optionally add a kart→eye sphere cast for walls; CTR has none.

---

## 6. Per-frame pseudo-code (30 Hz, units lu / 4096-angle; convert to radians at the end)

```rust
struct CtrCam {
    cms: i32,            // cameraMoveSpeed, |speedApprox| units (1/256 lu/frame)
    roll_s: i32,         // desiredRot.x, smoothed kart roll (4096 units)
    dpo: i32,            // damagePitchOffset, starts 0, goes to -32
    turbo_off: i32,      // fireSpeedZoom.distanceOffset (lu*256)
    turbo_timer: i32,    // CTR-ms
    cur_off: f32,        // heightSmoothing.currentOffset
    start_off: f32,      // heightSmoothing.startOffset
    blend_n: i32,        // heightSmoothing.framesRemaining
    reset: bool,         // DIRECTION_CHANGED (true on spawn/respawn)
    look_back_prev: bool,
}

struct KartIn {
    pos: Vec3,             // lu (posCurr>>8), kart origin = bottom centre
    angle: i32,            // physics heading
    turn_angle_curr: i32,  // drawn yaw offset (rotCurr.y - angle - wobble)
    rot_w: i32,            // drift camera spin rotCurr.w (computed by physics, section 3.1)
    rot_z: i32,            // kart roll rotCurr.z (from ground normal, COLL.c:1584)
    speed_approx: i32,     // signed in-plane speed, 1/256 lu/frame
    spinning: bool,        // ACTION_WARP: spin-out / tumble / warp
    turbo_event: bool,     // VehFire_Increment fired this frame
    fire_speed_cap: i32,
    look_back: bool,       // R2
}

const D_MIN: f32 = 179.0;  const D_MAX: f32 = 224.0;   // Far: 224 / 288
const V_DIST: f32 = 64.0;  const LOOK_H: f32 = 112.0;  // Far: 96 / 96
const PITCH: i32 = -100;   // 2P: 0

fn update(c: &mut CtrCam, k: &KartIn, ground_y_below: impl Fn(Vec3) -> Option<(f32, bool /*water*/)>)
    -> (Vec3 /*eye*/, Vec3 /*look_at*/, f32 /*roll rad*/, f32 /*vfov deg*/)
{
    if k.look_back != c.look_back_prev { c.reset = true; }
    c.look_back_prev = k.look_back;
    let rev = if k.look_back { 2048 } else { 0 };

    // 2.2 speed filter -> boom length
    let x = k.speed_approx.abs();
    if c.reset { c.cms = x; }
    else if x < c.cms { c.cms = (240 * c.cms + x) >> 8; }
    else { c.cms = (240 * c.cms + 16 * x) >> 8; }
    let t = ((c.cms - 1024) as f32 / 4096.0).clamp(0.0, 1.0);
    let mut d = D_MIN + t * (D_MAX - D_MIN);

    // 3.3 turbo push-back
    if k.turbo_event { c.turbo_timer = c.turbo_timer.max(k.fire_speed_cap >> 4); }
    if c.turbo_timer == 0 { c.turbo_off = (c.turbo_off - 512).max(0); }
    else {
        c.turbo_off = (c.turbo_off + 2048).min(0x6000);
        c.turbo_timer = (c.turbo_timer - 32).max(0);
    }
    d += (c.turbo_off >> 8) as f32;

    // 2.3 eye
    let a_eye = k.rot_w + k.angle + rev;           // heading the boom trails
    let f = fwd(a_eye);                            // (sin, 0, cos)
    let r = right(a_eye);                          // (-cos, 0, sin)
    let (sp, cp) = (sin4096(-PITCH), cos4096(PITCH)); // 0.15280, 0.98826
    let roll2 = ang(2 * c.roll_s);                 // previous frame's roll_s, as the game does
    let side = r * (64.0 * roll2.sin()) + f * (64.0 * sp * roll2.cos());
    let base = Vec3::new(k.pos.x + side.x, k.pos.y, k.pos.z + side.z);
    let mut eye = base - f * (cp * d) + Vec3::Y * (sp * d + V_DIST);

    // 2.4 look-at
    c.dpo = if k.spinning { (c.dpo + 8).min(0) } else { (c.dpo - 8).max(-32) };
    let g = fwd(k.rot_w + k.angle + k.turn_angle_curr + rev);
    let ahead = -(c.dpo as f32);                   // 32
    let look = base + g * (cp * ahead) + Vec3::Y * (LOOK_H - sp * ahead);

    // 2.5 roll filter (after use, as in CAM.c:1288)
    c.roll_s = (224 * c.roll_s + 32 * k.rot_z) >> 8;

    // 5. ground clearance
    match ground_y_below(eye) {   // probe eye.y+256 .. eye.y-2048, camera-search quads only
        Some((gy, water)) => {
            let floor = if water { 0.0 } else { gy };
            eye.y = eye.y.max(floor + V_DIST);
            if c.blend_n > 0 {
                let n = c.blend_n as f32;
                eye.y = ((8.0 - n) * eye.y + n * (k.pos.y + c.start_off)) / 8.0;
                c.blend_n -= 1;
            }
            c.cur_off = eye.y - k.pos.y;
        }
        None => {
            c.blend_n = 8;
            if eye.y < k.pos.y + c.cur_off {
                eye.y = k.pos.y + c.cur_off;
                c.start_off = c.cur_off;           // cur_off left unchanged
            } else {
                c.start_off = eye.y - k.pos.y;
                c.cur_off = c.start_off;
            }
        }
    }

    c.reset = false;
    let screen_roll = ang(c.roll_s / 2);           // CAM.c:1494 uses the freshly updated roll_s
    (eye, look, screen_roll, 73.74)                // vertical FOV, Hor+ for 16:9
}
```
Notes:
- `rot_w` must be computed by your physics as in section 3.1. If your port lacks it, implement that 10-line rate limiter.
- `turn_angle_curr` should exclude `turnWobbleAngle`. With wobble included, the camera look-at will shimmy.
- The game uses whole-lu integer math. Floats are fine.
- Roll sign: the screen roll in PSX space is `+rollS/2` applied as `Rz` in a `Ry*Rx(0x800+pitch)*Rz` camera, and the 0x800 X flip mirrors it. In a Y-up right-handed renderer, check visually that the horizon tilts *with* the slope's lean, the same way the kart tilts, at half the kart's angle. If it is opposite, negate it. **Ambiguous.**
- The same check applies to the lateral `side` shift. Verify it moves toward the side the kart leans toward. If it looks wrong, negate it; the effect is small either way.
- Blasted handling (section 3.2) is omitted from the pseudo-code. Add it if you implement weapon hits.

---

## 7. Uncertainties

1. **Roll / side-shift sign** in your frame: derived from the matrix conventions, but not verified in-game (section 6 notes).
2. **Thread order** (camera before or after `VehFire_Increment` and the physics in the same frame). It can shift turbo and drift effects by 1 frame.
3. **`CAM_FOLLOW_DRIVER_QUAD_FLAGS_SKIP_TERRAIN_HEIGHT = 0x4100`** (`CAM.c:6`): 0x4000 is `NO_CAMERA_RESPAWN_PROBE`, and 0x0100 is unnamed. Which quads carry these flags is level data.
4. `terrainHeightFloor` is read from scratch `hitPos.y` (`namespace_Camera.h:20`, `namespace_Coll.h` QuadBlockColl `0x1C`). It is assumed to be the ground height under the eye after the probe. If the probe misses, the no-hit branch is used, so this is safe.
5. Physical display aspect (letterbox 216/240) vs the designed 4:3 frustum: the angular FOV is unaffected.
6. The upward look (look-at about 9 lu above the eye at full speed) is surprising but follows directly from `angle[2] = 112` > `0.153*224 + 64`. The projection centre is exactly mid-viewport, so there is no compensating offset.
