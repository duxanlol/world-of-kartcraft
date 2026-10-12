# Crash Team Racing (PS1, NTSC-U) kart driving physics: port spec

Source of truth: the decompiled game, a checkout of [CTR-native](https://github.com/CTR-tools/ctr-native).
All `file:line` references are relative to that repo. Where the repo has both a retail
(`#else`, inline asm) and a native (`#if defined(CTR_NATIVE)`) code path, the **native C path is
quoted**. It computes the same values in readable C.

The goal is a standalone Rust library that reproduces CTR's kart feel on arbitrary triangle-mesh
terrain (WoW ADT/WMO), so CTR's quadblock/BSP collision is described only as a *contract*
(section 9 and the collision part of section 2). Everything else is specified exactly.

---

## 0. Units, fixed point, timestep, coordinate frame

| Quantity | CTR representation | Notes / conversion |
|---|---|---|
| Frame rate | 30 fps (`include/macros.h:56` `#define FPS 30`) | One physics tick per rendered frame. |
| `gGT->elapsedTimeMS` | "CTR milliseconds" per frame; **nominal 32** | `MainFrame.c:188-202`: `elapsed = (timerTicks << 5) / 100`, clamped to `[0, 64]` (negative becomes 32), and forced to 32 on unpause. So **1 s = 960 CTR-ms** and 1 CTR-ms is about 1.0417 real ms. Every timer (reserves, turbo meter, coyote, etc.) is in CTR-ms. `UI_Meter.c:21` `UI_JUMP_METER_TIME_UNIT = 0x3c0` (960) confirms 960 units = 1 s. |
| Time scaling idiom | `x * elapsedTimeMS >> 5` | With ms = 32 this is exactly `x` per frame. **Port recommendation: fixed 30 Hz tick, ms = 32.** Then every "rate * ms >> 5" is simply "rate per frame". |
| Position `posCurr` | `Vec3` of s32, **1/256 level unit (lu)** | Level geometry is s16 in lu; collision uses `posCurr >> 8` (`COLL.c:1168-1170`, `COLL.c:2290-2298`). |
| Velocity `velocity`, `speed`, `speedApprox`, `baseSpeed` ... | **1/256 lu per frame** (at ms = 32) | Position step is `pos += ((vel * ms) >> 5) * frac >> 12` (`COLL.c:2169-2172`, `2276-2278`, `2329-2331`), i.e. `pos += vel` per frame. So speed `S` is `S/256` lu/frame, or `S*30/256` lu/s. Crash's top speed of 13140 is 51.3 lu/frame, about 1540 lu/s. |
| Accelerations / frictions (MetaPhys) | speed units per frame, scaled `* ms >> 5` | e.g. gravity 900 means vy -= 900 per frame (3164 lu/s²). |
| Angles | **4096 = 360°** (`include/ctr_math.h:93`) | u16/s16, masked `& 0xFFF`. |
| Normals, rotation matrices, trig | **4096 = 1.0** (`FRACTIONAL_BITS 12`, `ctr_math.h:118`) | Trig via a 1024-entry quarter table `data.trigApprox` (packed sin in the low 16 bits, cos in the high 16 bits). |
| "8.8" quantities | 256 = 1.0 | Terrain multipliers, `simpTurnState << 8` turn rates. |
| Angular rate `ampTurnState`, `rotationSpinRate` | 1/256 angle-unit per frame | `angle += amp * ms >> 13`. With ms = 32 that is `amp/256` angle units per frame. So `simpTurnState` (e.g. 28) is about angle units per frame (28/4096 · 360° · 30 ≈ 74°/s). |

**World length scale (UNCERTAIN).** Nothing in the code converts lu to meters. Useful sizes:
- kart collision sphere radius = 25 lu (`COLL.c:2117`), and its center is 25 lu above the kart origin (`VehPhysForce.c:937-944`; the origin is the bottom-center of the kart);
- a hop apex is about 46 lu (section 6).

A plausible guess is **1 lu ≈ 2.5 cm**: sphere diameter 1.25 m, top speed about 38 m/s (≈140 km/h). Make `meters_per_lu` a tunable.

**Coordinate frame.** CTR world space is right-handed and **Y-up** (gravity is `-Y`, `VehPhysForce.c:239`).
- Yaw `a` gives forward = `(sin a, 0, cos a)` (`VehPhysForce.c:44-90`, `VehPhysForce_RotAxisAngle` with an up normal). This is the standard right-hand rotation about +Y, so a positive yaw change is counter-clockwise seen from above.
- **Steering left increases the angle.**
- Kart-local frame (columns of `matrixMovingDir`): X = `normal × forward` (points to the kart's **left**), Y = ground normal, Z = forward.

For WoW (right-handed, Z-up, facing 0 = +X/north, counter-clockwise toward +Y/west), the handedness-preserving mapping is `wow = (ctr.z, ctr.x, ctr.y)`. CTR yaw then maps directly to the WoW facing angle (`facing_rad = angle * 2π / 4096`).

**Integer semantics.**
- `CTR_MipsSra` is an arithmetic shift (floor).
- `VehCalc_MapToRange` (`VehCalc.c:36-59`) is `if v<=oldMin → newMin; if v>=oldMax → newMax; else newMin + (v-oldMin)*(newMax-newMin)/(oldMax-oldMin)` with C truncating division. Note that the clamps test `oldMin`/`oldMax` literally, so a reversed (negative) `oldMax` saturates immediately (this matters in drift code).
- `VehCalc_InterpBySpeed(val, step, target)` (`VehCalc.c:11-34`) moves `val` toward `target` by `step` without overshoot.
- `VehCalc_FastSqrt(n, shift)` returns `isqrt(n) << (shift/2)`. Physics calls it with shift 16, so `FastSqrt(x,16) >> 8 == isqrt(x)`.

---

## 1. Driver state that matters for driving

`struct Driver` is in `include/namespace_Vehicle.h:855-1938`.

| Field (offset) | Type | Meaning / units |
|---|---|---|
| `posCurr` (0x2D4), `posPrev` | Vec3 s32 | Kart origin (bottom center), 1/256 lu. `posPrev` is saved at the start of `PhysLinear` (`VehPhysProc.c:679-681`). |
| `velocity` (0x88) | Vec3 s32 | World velocity, speed units (1/256 lu/frame). Rebuilt every frame from `speed` + `axisRotationX/Y` (`VehPhysForce.c:946`). |
| `speed` (0x38C) | s16 | 3D speed magnitude, ≥ 0, clamped ≤ 25600 (`VehPhysForce.c:929-932`). |
| `speedApprox` (0x38E) | s16 | **Signed in-plane speed**: magnitude of velocity minus its component along the kart's up axis, negative if it points backward relative to the kart's forward (`VehPhysCrash.c:85-101`). Used for nearly every speed test. |
| `jumpHeightCurr` (0x390) | s16 | Signed velocity component along the kart up axis (`VehPhysCrash.c:73-82`). Visual squash and V-shift checks use it. |
| `axisRotationX` (0x396) | s16 angle | **Yaw of the velocity vector** (`ratan2(vx, vz)`, `VehPhysCrash.c:71`). |
| `axisRotationY` (0x394) | s16 angle | **Pitch of the velocity vector** (`ratan2(vy<<8, |vxz|<<8)`, `VehPhysCrash.c:70`). |
| `angle` (0x39A) | s16 angle | **Kart physics heading** (forward axis of `matrixMovingDir`). |
| `turnAngleCurr/Prev` (0x3C6/8) | s16 | Visual yaw offset of the model relative to `angle` (drift angle, oversteer look). It feeds back into physics via the counter-steer nudge (section 4.6). |
| `turnAngleLerpVel` (0x3D2) | s16 | Velocity of `turnAngleCurr` in normal driving. |
| `rotCurr.y` | s16 | Visual yaw = `angle + turnAngleCurr + turnWobbleAngle`. |
| `rotCurr.w`/`rotPrev.w` | s16 | Camera spin offset during drift (visual). |
| `simpTurnState` (0x4B) | s8 | Signed steer command this frame (+ = left), magnitude ≤ the turn strength (28, 64, 90 ...). |
| `rotationSpinRate` (0x3B4) | s16 | Smoothed turn rate (`simpTurnState<<8` target) in normal drive; in drift it is the steering spin component. |
| `ampTurnState` (0xC0) | s16 | **Final yaw rate applied this frame** (1/256 angle unit per frame). |
| `numFramesSpentSteering` (0x3E6) | s16 | Frames since the steer direction started (10000 = not steering). |
| `baseSpeed` (0x39C) | s16 | **Target speed** for this frame (signed; negative = reverse), after terrain scaling and turn penalty. |
| `fireSpeed` (0x39E) | s16 | Target speed before terrain scaling (`= gasTargetSpeed`). Used for the slope speed clamp. |
| `terrainScaledBaseSpeed` (0x3C4) | s16 | Full-throttle target × terrain speed multiplier. |
| `forwardAccelImpulse` (0x3B2), `forwardAccelVector` | s16 | Last frame's engine accel (per-frame units); used by the slope logic. |
| `forwardDir` (0x3E8) | s16 | +1 / -1 travel direction (steering inverts when reversing). |
| `reserves` (0x3E2) | s16 | **Turbo time left**, CTR-ms. While > 0 the target speed is raised (section 7). |
| `fireSpeedCap` (0x3E4) | s16 | Extra top speed granted while `reserves > 0`. |
| `turbo_outsideTimer` (0x3DE) | s16 | "Already granted" reserves for pads/items (prevents stacking). |
| `turbo_MeterRoomLeft` (0x3DC) | s16 | Powerslide meter: CTR-ms remaining until full (0 = full / idle). |
| `multDrift` (0x3DA) | s16 | Drift base turn rate, signed by drift direction (`(DriftTurnBase)<<8`). |
| `previousFrameMultDrift` (0x3EA), `timeUntilDriftSpinout` (0x3EC) | s16 | Post-drift turn assist (section 5.7). |
| `KartStates.Drifting.*` (0x580) | union | `numFramesDrifting` (spin-out counter), `driftBoostTimeMS`, `driftTotalTimeMS`, `numBoostsAttempted`, `numBoostsSuccess`. |
| `jump_TenBuffer` (0x3F0) | s16 | Jump input buffer in **frames** (10). |
| `jump_CooldownMS` (0x3F2) | s16 | 384 after a jump. |
| `jump_CoyoteTimerMS` (0x3F4) | s16 | 160 while grounded, counts down in air. |
| `jump_ForcedMS` (0x3F6) | s16 | 160 after a jump; re-applies the jump while still touching ground. |
| `jump_InitialVelY` (0x3F8) | s16 | Jump impulse to apply. |
| `jump_LandingBoost` (0x3FC) | s16 | Airtime accumulator (CTR-ms), reset on ground. |
| `jumpMeter` (0x46) | s16 | Hang-time meter shown in the HUD, used for the hang-time boost. |
| `wallRubTimer` (0x3FE), `wallRubSpeedLimit` (0x38A) | s16 | Wall-scrape slowdown window (240 ms, limit 3328). |
| `terrainFrictionTimer` (0x414) | s16 | Landing / ice-exit lateral grip boost timer. |
| `currentTerrain` (0xC2), `terrainMeta1` (0x358), `terrainMeta2` (0x35C) | | Terrain under the kart (`NONE` in air; `meta2` keeps the ground terrain up to 128 lu high). |
| `AxisAngle1_normalVec` (0x360) | SVec3 | **Raw ground normal** of the touched triangle this frame (reset to up each frame). Builds `matrixMovingDir`. |
| `AxisAngle2_normalVec` (0x368) | SVec3 | **Smoothed normal** (visual orientation `matrixFacingDir`). |
| `AxisAngle4_normalVec` | SVec3 | Previous frame's normal (jump calculation). |
| `matrixMovingDir` (0x310) | MATRIX | Physics frame: Y = AxisAngle1, Z = forward(angle). |
| `matrixFacingDir` (0x330) | MATRIX | Visual frame: Y = AxisAngle2, yaw = `rotCurr.y`. |
| `accel` (0x3CC) | SVec3 | Per-frame positional nudge (counter-steer); added before the sweep, removed after. |
| `actionsFlagSet` (0x2C8), `actionsFlagSetPrevFrame` | u32 | Flags (`namespace_Vehicle.h:580-613`). Important ones: `TOUCH_GROUND 0x1`, `STARTED_TOUCH_GROUND 0x2`, `JUMP_BUTTON_HELD 0x4`, `ACCEL_PREVENTION 0x8` (brake), `STEER_LEFT 0x10`, `BRAKE_WITH_ACCEL 0x20`, `JUMP_STARTED 0x400`, `DRIVING_AGAINST_WALL 0x2000`, `REVERSING_ENGINE 0x20000`, `AIRBORNE 0x80000` (a hop-initiated airtime that counts for hang time), `ACCEL_RELEASED_WITH_RESERVES 0x400000`, `MASK_WEAPON 0x800000`. |
| `kartState` (0x376) | u8 | `KS_NORMAL 0, KS_CRASHING 1, KS_DRIFTING 2, KS_SPINNING 3, KS_ENGINE_REVVING 4, KS_MASK_GRABBED 5, KS_BLASTED 6, KS_ANTIVSHIFT 9, KS_WARP_PAD 10, KS_FREEZE 11` (`namespace_Vehicle.h:57-69`). |
| `numWumpas` (0x30) | s8 | 0..10; adds a small top-speed bonus. 10 = "juiced". |
| `accelConst`, `turnConst`, `turboConst` (0x33-0x35) | s8 | Beta "Super Engine" stat tweaks. **Never written in retail** (no assignment anywhere in `game/`), so they are **always 0**. The formulas below keep them for completeness, but you can drop them. |
| `const_*` (0x416-0x484) | | Per-class constants copied from MetaPhys at spawn (section 10). |

---

## 2. Per-frame update order and state machine

### 2.1 Stage order

`MainFrame.c:308-330` runs, **for each stage, all drivers**, in `DriverFuncSlot` order (`namespace_Vehicle.h:19-34`):

```
0 INIT            one-shot state-entry hook (may be NULL)
1 UPDATE          state transitions (drift entry/exit, spin end, ...)
2 PHYS_LINEAR     timers, input -> baseSpeed/fireSpeed, steer command
3 AUDIO
4 PHYS_ANGULAR    yaw integration, velocity-heading rotation, matrixMovingDir
5 APPLY_FORCES    VehPhysForce_OnApplyForces: speed->velocity, gravity, friction, nudge
6 COLL_MOVED      COLL_MOVED_PlayerSearch: swept-sphere move + slide (position integration)
7 COLLIDE_DRIVERS VehPhysForce_CollideDrivers: remove nudge, turbo pads, kart-kart, pushback
8 COLL_FIXED      COLL_FIXED_PlayerSearch: ground probe, grounded flags, terrain, normals
9 JUMP_FRICTION   VehPhysGeneral_JumpAndFriction: turn penalty, engine accel, jump, velocity->speed
10 TRANSLATE_MATRIX visual matrix (matrixFacingDir), squash/stretch
11 ANIMATE, 12 PARTICLES   visual
```

`UI_JumpMeter_Update` (the hang-time boost) runs later the same frame from `UI_RenderFrame.c:143` (players only, unpaused).

### 2.2 Function tables per state

| State | UPDATE | PHYS_LINEAR | PHYS_ANGULAR | JUMP_FRICTION | Set by |
|---|---|---|---|---|---|
| Normal (`KS_NORMAL`) | `VehPhysProc_Driving_Update` | `VehPhysProc_Driving_PhysLinear` | `VehPhysGeneral_PhysAngular` | `VehPhysGeneral_JumpAndFriction` | `VehPhysProc_Driving_Init` (`VehPhysProc.c:1701-1737`; sets `turbo_MeterRoomLeft = 0`) |
| Powerslide (`KS_DRIFTING`) | `VehPhysProc_PowerSlide_Update` (installed by INIT on the next frame, `:2585-2590`) | `VehPhysProc_PowerSlide_PhysLinear` (= Driving_PhysLinear + skid flags, `:2577-2582`) | `VehPhysProc_PowerSlide_PhysAngular` | same | `VehPhysProc_PowerSlide_Init` (`:2593-2641`) |
| Wall slam (`KS_CRASHING`) | no-op | Driving_PhysLinear then `baseSpeed=fireSpeed=0` | `SlamWall_PhysAngular` (decays `turnAngleCurr`) | same | `VehPhysProc_SlamWall_Init` (`:2712-2763`); returns to Driving at animation end (`:2681-2709`) |
| Spin-out (`KS_SPINNING`) | SpinFirst → SpinLast → SpinStop | Driving_PhysLinear + `baseSpeed=0`, `ACCEL_PREVENTION` | spin (`:2812-2848`, `:2940-3012`) | same | `VehPhysProc_SpinFirst_Init` (`:2859-2909`) |
| Anti-V-shift freeze (`KS_ANTIVSHIFT`) | `FreezeVShift_Update` | Driving_PhysLinear | General | `FreezeVShift_ReverseOneFrame` | `VehPhysProc_FreezeVShift_Init` (`:1847-1870`) |

APPLY_FORCES, COLL_MOVED, COLLIDE_DRIVERS, COLL_FIXED and TRANSLATE_MATRIX are the same functions in every driving state.

Airborne is **not a separate state**. It is `ACTION_TOUCH_GROUND == 0` inside the same state.

### 2.3 Transitions (driving-relevant)

- **Normal → Drift** (`VehPhysProc.c:1643-1673`, evaluated in UPDATE, so it uses last frame's flags and inputs). All of these must hold:
  - `ACTION_STARTED_TOUCH_GROUND` (just landed, typically from a hop);
  - `|simpTurnState| > (TurnRate + turnConst*2/5) >> 1` (Balanced: > 14);
  - `buttonsHeld & buttonUsedToStartDrift` (the shoulder button that started the hop is still held);
  - not `ACCEL_PREVENTION` (not braking);
  - `speedApprox >= const_Speed_ClassStat >> 1`.
- **Drift → Normal** (`:2553-2573`) when any of these holds:
  - `numFramesDrifting < -FramesTillSpinout(60)`;
  - `speed < Speed_ClassStat/2`;
  - flags `DRIVING_AGAINST_WALL | ACCEL_PREVENTION | BRAKE_WITH_ACCEL`;
  - the drift button is released.

  On exit it calls `PowerSlide_Finalize` (`:2466-2470`) and then `Driving_Init`.
- **Drift → Spin**:
  - `numFramesDrifting > 60` (`NoInputTimer = 0x3C0`), `:2543-2547`;
  - `speedApprox < 0` and not on a turbo pad (`NoInputTimer = 0x780`), `:2548-2552`.
- **Any → SlamWall**: hard wall impact (section 9).
- **Normal → FreezeVShift**: `vShiftCount >= 5` after the 96 ms start guard (`:1678-1697`). This is an anti-jitter guard for V-shaped quad seams. Optional on arbitrary meshes; see section 9.

---

## 3. Throttle, brake, reverse, top speed, coasting

### 3.1 Inputs (`VehPhysProc.c:767-1031`)

- Cross (X) = gas, Square = brake, Circle = item, R1/L1 = hop/drift/boost.
- Left stick X = steer. The D-pad snaps `stickLX`/`stickLY` to 0 or 255 immediately (`GAMEPAD.c:431-501`, `672-709`).
- Down on the left stick/D-pad is used for reverse. The analog right stick Y (`stickRY`) is an analog throttle (neGcon/wheel); digital pads keep it at 128.

**Reserve handling:**
- Holding **Square** zeroes `reserves` unless on a turbo pad (`:954-963`).
- If `reserves != 0`, **Cross is treated as held** (`:975-1031`). Turbo auto-accelerates. If Cross is not physically held and `-ReturnToRest(stickRY) < 1`, the flag `ACCEL_RELEASED_WITH_RESERVES` is set (it only blocks Super Engine).
- If on a turbo pad, Square is ignored.

### 3.2 Target speed `gasTargetSpeed` (`:1036-1275`)

`B = VehPhysGeneral_GetBaseSpeed(d)` (`VehPhysGeneral.c:1979-2116`):
```
netStat   = ((AccelSpeed_ClassStat - Speed_ClassStat) << 12) / 5 - 1     // = 1228799 for every class (diff is 1500)
bonus     = (min(numWumpas,9) * netStat / 10 + min(turboConst,5) * netStat) >> 12   // 0..269
if MASK_WEAPON: bonus += MaskSpeed (3900)
B = Speed_ClassStat
if reserves != 0:
    B += fireSpeedCap
    cap = SingleTurbo + 2*(SacredFire - SingleTurbo) - fireSpeedCap      // = 6144 - fireSpeedCap
    bonus = min(bonus, max(cap, 0))
B += bonus
subtract = 0
if TNT on head: subtract = DamagedSpeed>>1;  if burn/squish/raincloud-slow: subtract = DamagedSpeed (4096)
if clockReceive: subtract = max(subtract, DamagedSpeed*(20 - rank) >> 4)
B = min(B - subtract, 0x6400)
```
(the constants are at `VehPhysGeneral.c:1966-1975`).

Digital-pad decision table (`ReturnToRest(v,128)` maps `|v-128|` from [48,127] to [0,128], `VehPhysJoystick.c:15-44`):

| Square | Cross | Down (stickLY ≥ ~100 after mapping) | gasTargetSpeed | flags |
|---|---|---|---|---|
| no | yes | – | `B` | clear `REVERSING` |
| no | no | no | `0` (coast) | clear `REVERSING` |
| no | no | yes (or >0 while already reversing) | `-BackwardSpeed` (-5120) | `REVERSING` (`:1207-1217`) |
| yes | yes | no | `B >> 1` | `BRAKE_WITH_ACCEL` (`:1112-1123`) |
| yes | – | yes | `-(3*BackwardSpeed)>>2` (-3840) | `REVERSING \| BRAKE_WITH_ACCEL` (`:1098-1110`) |
| yes | no | no | `0` | **`ACCEL_PREVENTION`** = brake (`:1162-1164`) |

Analog `stickRY` (rare): push up gives `B*s>>7`; pull back gives reverse `BackwardSpeed*s>>7`, or brake/reverse-steer latching if `speedApprox ≥ 0x301` (`:1236-1272`).

Then (`:1335-1388`):
```
fireSpeed = gasTargetSpeed               (accel-tap counter: sign change from <=0 to >0 within 256 ms increments accelTapCount)
if !(MASK_WEAPON|BRAKE_WITH_ACCEL) and terrainMeta2.speedMultiplier != 256:
    gasTargetSpeed = gasTargetSpeed * mult >> 8 ;  B = B * mult >> 8
terrainScaledBaseSpeed = B ; baseSpeed = gasTargetSpeed
```

### 3.3 Turn speed penalty (`VehPhysGeneral.c:1109-1155`, start of JUMP_FRICTION)

Applied only if not drifting, no mask, and `reserves == 0`:
```
t = |(s8)(ampTurnState >> 8)|                       // ~ |simpTurnState|
dec = min(MapToRange(t, 0, BackwardTurnRate(48), 0, TurnDecreaseRate), |baseSpeed|)
baseSpeed -= sign(baseSpeed) * dec
```
At full steer: Balanced -2380, Accel -4918, Speed -2833, Turn -4532.

Wall rub (`:1157-1173`): if `wallRubTimer != 0`, clamp `baseSpeed` to `±wallRubSpeedLimit` (3328).

### 3.4 Engine acceleration (`VehPhysGeneral.c:1181-1253`), only when `TOUCH_GROUND`

```
acc = 0
if on turbo pad and baseSpeed > 0:            acc = 8000
elif baseSpeed != 0:
    sliding_wrong_way = |speedApprox| > 0x2FF && (base<=0 || sA<=0) && (base>=0 || sA>=0)
        // i.e. moving fast opposite to the requested direction
    if sliding_wrong_way && !(terrain.flags & ACCEL_WHILE_REVERSE_SLIDING(0x4) && base>0 && sA<0):
        acc = 0                                  // brake via friction instead (3.6)
    else:
        acc = Accel_ClassStat + accelConst*32/5
        if on turbo pad: acc = (base>0) ? 8000 : acc (unscaled)
        else:
            if reserves != 0 && base > 0: acc = Accel_Reserves (1152)
            if terrain.slowUntilSpeed != 256 && !MASK: acc = acc * slowUntilSpeed >> 8
                // the field is misnamed; it is an ACCEL multiplier (dirt 180, ice 100, water 200)
acc = acc * ms >> 5
```

### 3.5 Applying acceleration and the speed cap (`:1255-1348`, `1576-1584`)

```
fwd = matrixMovingDir * (0, 0, acc)                // kart forward, in the ground plane
if baseSpeed >= 0: movement = velocity + fwd ; forwardAccelImpulse = acc
else:              movement = velocity - fwd ; forwardAccelImpulse = -acc
excess = isqrt(|movement|²) - |baseSpeed|           // FastSqrt(...,16) >> 8
speedLoss = clamp(excess, 0, acc)
... jump handling may raise movement.y (section 6) ...
ConvertVecToSpeed(movement)  -> speed=|movement|, axisRotationX/Y, speedApprox, jumpHeightCurr
speed = max(speed - speedLoss, 0)
```
Consequences:
- Below `|baseSpeed|`, speed rises by about `acc` per frame (minus friction).
- Above it, the engine contribution is cancelled exactly (no extra drag), and forward friction pulls the speed down.
- Equilibrium on flat asphalt with gas = `baseSpeed` (friction 120 < acc).
- Effective net acceleration at full gas ≈ `acc - PedalFriction_Forward` = 480 - 120 = 360/frame for Balanced. 0 → 13140 takes about 37 frames (1.2 s).

### 3.6 Ground friction (`VehPhysForce.c:429-806`, inside APPLY_FORCES)

Friction is applied in the kart-local frame (X lateral, Z forward). It runs if:
- the kart was on the ground last frame, or
- it is `KS_BLASTED`, or
- `speedApprox > terrainScaledBaseSpeed` while `terrainMeta2.speedMultiplier < 256` (slow-terrain drag also slightly airborne).

| Condition | perpendicular (X) | forward (Z) |
|---|---|---|
| `ACCEL_PREVENTION` (brake) | BrakeFriction 260 (raincloud: ×16; blasted: ×3/4; spinning: /2) | same |
| `baseSpeed != 0`, moving > 0x300 opposite to base | PedalFriction_Perp 350 | **BrakeFriction 260** |
| `baseSpeed != 0`, drifting | **DriftCurve 120** | **DriftFriction 120** |
| `baseSpeed != 0`, normal | PedalFriction_Perp 350 | PedalFriction_Fwd 120 |
| `baseSpeed == 0` (coast) | NoPedalFriction_Perp 425 | NoPedalFriction_Fwd 120 (raincloud heavy: both BrakeFriction×16) |

Then:
```
perp = perp*ms>>5 ; fwd = fwd*ms>>5
if terrain.groundFrictionScale != 256: perp = perp*gfs>>8 ; fwd = fwd*gfs>>8       // ice 32, icy road/snow 128, mud 512
terrainFrictionTimer:
   > 0 (landing, 320 ms): perp += perp*TerrainFrictionBoost(256)>>8  (i.e. x2), count down
   < 0 (just left ice, -320): first frame perp = |localX|/2, else perp x2, count up to 0
MUD flag (0x80): perp = max(perp, |localX>>3|); fwd = max(fwd, |localZ - base|/2 or |localZ|/2 when opposing)  (:683-785)
SIDESLIP flag (0x100): perp = max(perp*3/4, fwd)
localX = InterpBySpeed(localX, perp, 0) ; localZ = InterpBySpeed(localZ, fwd, 0)
```
Coasting from top speed on asphalt: -120/frame, about 110 frames (3.6 s) to stop. Braking: -260/frame (about 1.7 s).

### 3.7 Reverse

Reverse target is -5120 (Square+Down: -3840). PhysAngular flips the steer sign when `forwardDir < 0` (`VehPhysGeneral.c:160-207`), so the kart turns like a car in reverse.

---

## 4. Steering

### 4.1 Steer command (`VehPhysProc.c:1391-1534`, PHYS_LINEAR)

```
strength = TurnRate + turnConst*2/5                   // Balanced 28, Accel 26, Speed 24, Turn 30
if accelTapCount >= 7 and |speedApprox| < 0x2600:   strength = 90     // X-mashing trick
elif wallRubTimer != 0:                               strength = 48
elif ACCEL_PREVENTION|BRAKE_WITH_ACCEL:
    if Cross held (power-brake):  strength = MapToRange(|speed|, 0x300, Speed_ClassStat/2, 64, strength)
    else (brake only):            strength = 64
simpTurnState = -GetStrengthAbsolute(stickLX, strength)   // + = left
```
`GetStrength` (`VehPhysJoystick.c:46-99`) is a two-segment curve:
- dead zone 48;
- the first half of travel (47 counts) ramps 0 → strength/5;
- the second half ramps strength/5 → strength;
- `|stick-128| ≥ 127` gives the full strength (D-pad = full).

Direction tracking (`:1500-1534`):
- `simpTurnState == 0` sets `numFramesSpentSteering = 10000`.
- A sign change (or starting from 0) toward left sets `STEER_LEFT`, toward right clears it, and either resets `numFramesSpentSteering = 0`.

### 4.2 Turn-rate response (`VehPhysGeneral.c:160-375`, PHYS_ANGULAR)

```
target = simpTurnState << 8 ; if forwardDir < 0: target = -target, toggle STEER_LEFT
if TOUCH_GROUND and not on turbo pad:
    target = MapToRange(|speedApprox|, 16, 0x300, 0, target)        // no steering when stopped
k = terrain.turnResponseScale   (asphalt 256, ice 32)
if target == 0:            rate -> 0      by (TurnInputDelay + turnConst*50) * k >> 8 per frame
elif |rate| < |target|:    rate -> target by (TurnInputDelay + turnConst*100) * k >> 8 per frame
else:                      rate -> target by (TurnInputDelay + turnConst*50) * k >> 8 per frame
rotationSpinRate = rate ; rot = rate
```
`TurnInputDelay` = 5000 / 4500 / 4000 / 5500 per frame, so on asphalt the response is nearly instant (≤ 2 frames). On ice (×32/256) it takes about 11 frames.

### 4.3 Post-drift assist (`:378-415`)

If `timeUntilDriftSpinout != 0`: `rot += MapToRange(t, 0, 320, 0, previousFrameMultDrift)`, then `t -= ms` (clamp at 0). This is a residual drift-direction turn that fades over the last 320 of 480 ms after a drift ends.

### 4.4 Steering ramp ("steer accel", `:633-727`)

Applied if `|speedApprox| ≥ 0x300` and on ground:
```
sa = SteerAccel(numFramesSpentSteering, s2first=4, s2len=8, s4first=64, min=0, max=3072)
     // frames 0..4: 0->3072 ; 4..12: 3072 ; 12..64: 3072->0 ; >64: 0   (VehCalc.c:61-99)
numFramesSpentSteering++
sa = min(sa, SteerAccelTurnVelScale(128) * |rot| >> 8)        // <= |rot|/2
if STEER_LEFT: sa = -sa                                         // always opposes rot
rot += sa, but never crossing 0 (TurnVelLimit = 0)
```
Net effect: a fresh steer input turns at **about half rate for roughly the first 0.4 s**, then ramps up to the full rate by frame 64. (Code comment: "the higher steerAcceleration the more steering is locked up".)

### 4.5 Yaw and velocity-heading integration (`:858-934`)

```
ampTurnState = rot
angle = (angle + (rot*ms >> 13)) & 0xFFF
leanRot = rot
if ACCEL_PREVENTION or accelTapCount >= 7: leanRot = rot*10 >> 8     // braking: velocity barely turns (slides)
elif terrain.turnLeanScale != 256:          leanRot = rot*turnLeanScale >> 8   // ice: 0
axisRotationX = (axisRotationX + (leanRot*ms >> 13)) & 0xFFF          // rotate the VELOCITY heading with the kart
matrixMovingDir = RotAxisAngle(AxisAngle1_normalVec, angle)
CounterSteer()
```
This is the core "grip" model. On normal terrain the velocity vector is rotated by the same yaw as the kart, and lateral friction (350/frame) removes any residual side slip. On ice the velocity is not rotated, and only the weak friction (350×32/256 = 44/frame) bends it.

### 4.6 Visual model yaw (`turnAngleCurr`) and its two physics couplings (`:417-632`, `959-1033`, `817-857`, `VehPhysForce.c:2054-2135`)

```
resistMin = 32*Speed_ClassStat>>8 ; resistMax = 224*Speed_ClassStat>>8   (halved if BRAKE_WITH_ACCEL, which also uses
     modelRotVel = MapToRange(|speed|, 0x300, ClassSpeed/2, ModelRotVelMin 450, ModelRotVelMax 350), else 350)
full = (TurnRate + turnConst*2/5) << 8
turnResistance = MapToRange(|speed|, resistMin, resistMax, full, 0)
lerpTarget = 0
if |speedApprox| >= resistMin and |rot| > turnResistance:
    lt = MapToRange(|fireSpeed|, resistMin, resistMax, 0, modelRotVel)
    lerpTarget = sign(rot) * MapToRange(|rot|, turnResistance, full, 0, lt)
turnAngleLerpVel = LerpToForwards(turnAngleCurr, turnAngleLerpVel, lerpTarget)
   // mirror so target>=0; desired = 0 if wallRubTimer == 240, else
   //   target < cur : -min(cur>350 ? 20*15 : 20, max(1,(cur-target)>>2))
   //   cur < target : +min(cur<0 ? 60 : 30, max(1,(target-cur)>>2))
   // vel = InterpBySpeed(vel, 10, desired)
turnAngleLerpVel *= terrain.turnAngleScale/256 (ice 128)
turnAngleCurr += turnAngleLerpVel*ms >> 5
```
Physics couplings:
1. **Low-speed transfer** (`:841-857`): `d = MapToRange(|speedApprox|, 0, 0x600, turnAngleLerpVel, 0)*ms>>5`; if `|d| > 1`: `angle -= d`.
2. **Counter-steer nudge** (`VehPhysForce_CounterSteer`, `VehPhysForce.c:2054-2135`, called at the end of both PhysAngulars). If `|speedApprox| > 768`, not crashing, not `ACTION_WARP`, `wallRubTimer == 0`, on ground, and `terrain.counterSteerRatio != 0`:
   ```
   v = (-8000 * counterSteerRatio) >> 8
   dA = clamp(turnAngleCurr - turnAnglePrev, ±ModelTurnCounterSteerStrength(30))
   accel = matrixMovingDir * ((v * sin(dA)) >> 12, 0, 0)
   ```
   `accel` is added to velocity before the sweep and removed after it (`VehPhysForce.c:963-979`, `986-988`), so it is a **one-frame lateral position offset**, not momentum. Magnitude ≤ about 370 (1.4 lu).

Wobble (`turnWobbleAngle/Velocity/Timer`, `:728-816` and drift `:2262-2372`) is purely visual: rotCurr.y and rumble.

---

## 5. Powerslide

### 5.1 Entry (hop + steer)

1. Tapping R1 or L1 (not while drifting) records `buttonUsedToStartDrift` (both pressed counts as R1), sets `jump_TenBuffer = 10` frames and sets `JUMP_BUTTON_HELD` (`VehPhysProc.c:904-949`). Raincloud `JUMP_LOCKOUT` blocks this.
2. JUMP_FRICTION performs the hop (section 6).
3. On the landing frame (`STARTED_TOUCH_GROUND`), `Driving_Update` checks the conditions in 2.3 and calls `PowerSlide_Init`.

**`PowerSlide_Init`** (`:2593-2641`):
```
kartState = KS_DRIFTING
multDrift = (DriftTurnBase + turnConst*4/5) << 8, negated if simpTurnState < 0  // Bal 3584, Acc 2560, Spd 1280, Trn 4608
numFramesDrifting = driftTotalTimeMS = driftBoostTimeMS = numBoostsAttempted = numBoostsSuccess = 0
rotationSpinRate = 0 ; turnAngleLerpVel = 0 ; timeUntilDriftSpinout = 0
turbo_MeterRoomLeft = TurboMaxRoom(30) << 5 = 960   // meter starts empty, fills over 960 CTR-ms (1 s)
```

### 5.2 Drift angular update (`VehPhysProc_PowerSlide_PhysAngular`, `:1873-2464`)

Steps in order:

```
// (a) velocity heading and kart heading converge (:1912-1951)
delta = ((axisRotationX - angle + 2048) & 0xFFF) - 2048
if delta != 0:
    step = delta >> 3 ; if step == 0: step = 1
    step = clamp(step, ±(ms*2))                 // ±64 per frame
    angle += step ; axisRotationX -= step        // both move toward each other

// (b) camera spin offset rotCurr.w -> ±DriftCameraSpinRate(384)   (visual, :1953-2007)

// (c) steering spin component (:2009-2088)
range = (TurnRate + turnConst*2/5) << 8
s = simpTurnState << 8
if multDrift >= 0 (drift left):
    desired = s>=0 ? Map(s, 0, range, 0,  DriftStandard(22)<<8)        // into drift: up to +5632
                   : Map(-s,0, range, 0,  DriftSwitchWay(-10)<<8)      // counter-steer: down to -2560
else (drift right): mirror: s<=0 -> Map(-s,..., -(22<<8)) ; s>0 -> Map(s,..., +(10<<8))
// rate limit (signs mirrored so desired >= 0):
if desired < current: current -= DriftSpinRateDecel(500)*ms>>5, not below desired
else:                 current += DriftSpinRateAccel(3000)*ms>>5, not above desired

// (d) spin-out counter (:2090-2121), counted per frame
if desired == 0:                       numFramesDrifting -> 0 by 1
elif steering into the drift:          numFramesDrifting = max(numFramesDrifting+1, 0)
else (counter-steering):               numFramesDrifting = min(numFramesDrifting-1, 0)

// (e) drift base rate ramps in over DriftTurnRampFrames(60)<<5 = 1920 CTR-ms (:2132-2139)
dti = Map(driftTotalTimeMS, 0, 1920, (DriftTurnStartupScale(100)*multDrift)>>8, multDrift)  // starts at 39%
// (f) the spin may not reverse the turn direction (:2142-2160)
if dti >= 0: current = max(current, -dti) else: current = min(current, -dti)
rotationSpinRate = current

// (g) visual drift angle target (:2162-2260)
base = ±DriftTurnAngleScale(768)    // MapToRange(|dti|, 0, DriftTurnBase+.., 0, 768) always saturates
target = base + (spin same sign as drift ?  Map(|spin|, 0, 22<<8, 0, ±300)
                                         :  ±(-128))    // counter-steer: reversed oldMax saturates -> constant -128
turnAngleCurr += max-1 step of (target - turnAngleCurr) >> 3

// (h) yaw (:2377-2382)
ampTurnState = current + dti
angle = (angle + (ampTurnState*ms >> 13)) & 0xFFF

// (i) boost kick (:2384-2430): while driftBoostTimeMS > 0 (480 after each successful boost):
driftBoostTimeMS -= ms
axisRotationX += sign(turnAngleCurr) * (DriftBoostAxisKickRate(36)*ms >> 5)     // +36 per frame into the drift

rotCurr.y = turnWobbleAngle + angle + turnAngleCurr
driftTotalTimeMS = min(driftTotalTimeMS + ms, 1920)
matrixMovingDir = RotAxisAngle(AxisAngle1, angle) ; CounterSteer()
```

Resulting yaw rates (per frame, ms = 32, fully ramped, `amp/256`):

| Class | neutral (dti only) | full into drift | full counter-steer |
|---|---|---|---|
| Balanced (base 14) | 14 | 36 | 4 |
| Accel (10) | 10 | 32 | 0 (clamped) |
| Speed (5) | 5 | 27 | 0 (clamped) |
| Turn (18) | 18 | 40 | 8 |

In units of angle per frame, 14 is 1.23°/frame, about 37°/s.

Linear motion in a drift:
- friction perpendicular/forward = DriftCurve/DriftFriction = 120/120 (vs 350/120 normally), so the kart slides;
- engine accel acts along `angle`;
- the turn speed penalty (3.3) is skipped;
- the velocity heading follows only via (a), (i) and friction. There is **no** lean-rotation of `axisRotationX` in drift.

### 5.3 Turbo meter and boosts (`VehPhysProc_PowerSlide_Update`, `:2473-2574`)

Constants: `TurboMaxRoom = 30` (meter length 30 frames = 960 ms), `TurboLowRoomWarning = 15` (the red zone is the last 480 ms), `TurboFullBarReserveGain = 60` (1920 ms), `DriftBoostDurationFrames = 15` (480 ms kick).

Each frame in drift:
```
if L1 or R1 TAPPED this frame:
    numFramesDrifting = 0                                   // pressing always resets the spin-out counter
    m = turbo_MeterRoomLeft
    if m != 0:
        if m < 480:   // red zone -> SUCCESS
            reserves_gain = Map(m, 0, 480, 1920, 0)         // = 1920 - 4*m  (more the closer to full)
            fire = numBoostsSuccess << 6                     // 0, 64, 128
            VehFire_Increment(d, reserves_gain, POWER_SLIDE_HANG_TIME, fire)
            numBoostsSuccess++ ; driftBoostTimeMS = 480
        else:         // pressed too EARLY (white/green part) -> FAIL ("backfire" exhaust puff)
            failedBoostExhaustTimer = 8
        turbo_MeterRoomLeft = 0
        numBoostsAttempted++
    // m == 0 (meter idle/full): the press does nothing except reset numFramesDrifting
else:
    if m != 0:
        m -= ms ; if m <= 0: m = 0, play "meter full" sfx, numBoostsAttempted += 3   // waited too LONG: chain over
    elif numBoostsAttempted < 3:
        turbo_MeterRoomLeft = 960                           // next cycle starts (next frame after a press)
```
So:
- A drift allows **at most 3 attempts**. Each attempt is a 960 ms fill; you must press in the last 480 ms.
- **Too early = fail.** The attempt is consumed and you get no reserves.
- **Too late** (the meter reaches full without a press) ends the chain.
- Successful boost *k* (k = 0, 1, 2) adds `1920 - 4*m` CTR-ms of reserves (max about 2 s) and raises `fireSpeedCap` to `2048 + 8*fire`: **2048 / 2560 / 3072**.

Example: a perfect triple at the last moment gives about +1920 ms each, and the speed cap rises to Speed + 3072 (+bonus).

### 5.4 Spin-out and exit

See 2.3:
- holding the steer into the drift for more than 60 frames (2 s) without pressing a shoulder button spins out;
- counter-steering for more than 60 frames ends the drift normally;
- slowing below half the class speed, hitting a wall, braking or releasing the hop button ends the drift.

### 5.5 Counter-steer behaviour summary

Counter-steering:
- maps the spin toward `SwitchWay*256` (-2560);
- is limited so that `amp` keeps the drift sign (minimum yaw = `dti - 2560`, or 0);
- sets the visual angle target to 768-128 = 640;
- counts `numFramesDrifting` down (exit at -61).

### 5.6 Wall slam during a drift

`COLL.c:2638-2645` folds `turnAngleCurr` into `angle` before entering SlamWall.

### 5.7 Exit residue

`PowerSlide_Finalize`: `previousFrameMultDrift = multDrift`, `timeUntilDriftSpinout = 15<<5 = 480` (section 4.3).

---

## 6. Hop, jumps, airborne, landing

### 6.1 Ground and coyote

`COLL.c:1466-1513`. When grounded:
- `jump_CoyoteTimerMS = 160` (`COLL.c:1096`);
- `jump_LandingBoost = 0`;
- `TOUCH_GROUND` is set.

When airborne (`COLL.c:1515-1544`):
- coyote -= ms; when it reaches 0, `jump_CooldownMS` and `jump_HighJumpTimerMS` are cleared;
- `jump_LandingBoost += ms`;
- `AIRBORNE` is set **only if `jump_CooldownMS != 0`**, i.e. a hop started within the last 384 ms. Driving off a ramp without hopping gives no hang-time credit.

### 6.2 Normal hop (`VehPhysGeneral.c:1423-1433`)

If `jump_CoyoteTimerMS != 0 && jump_TenBuffer != 0 && jump_CooldownMS == 0`:
`jump_ForcedMS = 160; jump_InitialVelY = JumpForce (4596)`. Then PROCESS_JUMP.

### 6.3 PROCESS_JUMP (`:1435-1530`)

```
jump_CooldownMS = 384 ; jump_TenBuffer = 0 ; flags |= JUMP_STARTED
best = argmax_abs( JumpGetVelY(AxisAngle4 /*prev frame normal*/, movement),
                   JumpGetVelY(TOUCH_GROUND ? AxisAngle1 : AxisAngle2, movement) )
   JumpGetVelY(n, v) = |n.y| < 21 ? 0 : (v.x*n.x + v.z*n.z) / n.y     // = -(slope-tangent vertical velocity)
vy = isqrt(best² + InitialVelY²) - best          // FastSqrt((b²+J²)>>8, 8)
cap = level.jumpVerticalSpeedCap << 8 ; if cap == 0: cap = 0x3700 (14080) ; cap = min(cap, 0x5000)
movement.y = max(movement.y, min(vy, cap))
```
On flat ground vy = 4596. Driving up a slope adds the slope's vertical velocity.

While still touching ground and `jump_ForcedMS > 0` (160 ms), PROCESS_JUMP is re-run every frame (`:1350-1367`), so the hop is guaranteed to leave the ground.

Variants:
- **Spring item** (`:1370-1390`): `InitialVelY = 9*JumpForce/4` (10341), sets `jump_HighJumpTimerMS = 384`.
- **Level forced jump**: `FORCED_JUMP_HIGH` gives `3*JumpForce` (13788); low gives `3*JumpForce/2` (6894) (`:1392-1421`).
- **Anti-gravity quads**: when not jumping and on a quad with `mulNormVecY != 0`, `movement += matrixMovingDir*(0, n*|speedApprox|>>8, 0)` (`:1532-1574`). This is CTR-specific; skip it.

### 6.4 Gravity (`VehPhysForce.c:237-310`, APPLY_FORCES)

- `g = -Gravity(900) * ms >> 5` per frame, in world Y.
- Quads with the low-gravity flag use `g*41/100` (`:259-276`).
- Gravity is transformed into the kart-local frame (`matrixMovingDir`ᵀ) and added to the local velocity:
  - `gZ` (along forward) is **dropped if it opposes `forwardAccelImpulse`**. While the engine pushes, you do not lose speed uphill.
  - `gX` and `gZ` are dropped while braking (`ACCEL_PREVENTION`) or when `baseSpeed` and `speedApprox` have opposite signs.
- Clamps on the gravity contribution (`:315-420`). Each clamp only limits how far gravity can push; it never reduces an existing larger value:
  - `localZ` ≤ `fireSpeed + SlopeForwardSpeedBonus(5120)` and ≥ `fireSpeed - 2560`. **Downhill can carry you up to 5120 above the target.**
  - `|localX| ≤ SideSpeedClamp(2560)`.
  - `|localY| ≤ TerminalVelocity(20480)` (mud: 256).

In the air `matrixMovingDir` is built from an up normal (AxisAngle1 is reset to `(0,4096,0)` each frame, `VehPhysForce.c:968-972`), so gravity is purely vertical. Air control: steering still integrates `angle` (rate scaled by nothing: the low-speed map is ground-only), and `axisRotationX` is rotated by `rot*leanScale` with the airborne terrain NONE (`turnLeanScale` 256). **In air the velocity heading turns with the kart.** There is no air friction.

Flat hop: apex ≈ 4596²/(2·900)/256 ≈ 45.8 lu after about 5 frames. Total airtime is about 10-11 frames (0.35 s), which is below the 640 hang-time threshold, so a flat hop gives no boost.

### 6.5 Landing

COLL_FIXED on the first grounded frame (`COLL.c:1499-1513`):
- sets `STARTED_TOUCH_GROUND`;
- sets `terrainFrictionTimer = 320` (lateral grip ×2 for 320 ms);
- the landing sound volume scales with `|Δvy|`.

The into-ground velocity component is removed by the ground "ROAD" scrub in the sweep (section 9). Horizontal speed is kept.

### 6.6 Hang-time boost (`UI_Meter.c:59-121`)

```
while AIRBORNE:  jumpMeter = min(jump_LandingBoost, 2400)
on the first non-AIRBORNE frame after AIRBORNE:
   if jumpMeter >= 640:            // >= 0.667 s airtime
       fire = 0 ; if jumpMeter >= 960: fire = 0x80 ; if jumpMeter >= 1440: fire = 0x100
       VehFire_Increment(d, 960, POWER_SLIDE_HANG_TIME, fire)     // +1 s reserves
```

| Airtime | Reserves | Fire level | `fireSpeedCap` |
|---|---|---|---|
| 0.667–1.0 s | 960 | 0 | 2048 |
| 1.0–1.5 s | 960 | 128 | 3072 |
| ≥ 1.5 s | 960 | 256 | 4096 |

---

## 7. Turbo / reserves

### 7.1 `VehFire_Increment(d, reserves, type, fireLevel)` (`VehFire.c:100-385`)

- Ignored for turbo pads while braking. Ignored entirely while spinning, blasted or mask-grabbed (`:120-155`).
- `newCap = SingleTurboSpeed + fireLevel*(SacredFireSpeed - SingleTurboSpeed) >> 8 = 2048 + 8*fireLevel` (`:298-301`).
- `fireSpeedCap = newCap` if `reserves == 0`, or `newCap > fireSpeedCap`, or (`fireSpeedCap > 4096` and not on a super pad). The cap only rises while you still have reserves.
- Reserves accounting (`:342-375`):
  - `FREEZE_RESERVES_ON_TURBO_PAD` type (pads, turbo item): if `turbo_outsideTimer < R` then `reserves += R - turbo_outsideTimer; turbo_outsideTimer = R`. While you stay on a pad this re-tops-up the 32 ms drained each frame, so reserves are frozen. A re-use only tops up to R.
  - `SUPER_ENGINE`: `reserves = max(reserves, R)`.
  - Otherwise (start boost, hang time, powerslide): `reserves += R` (stacks).

### 7.2 Effects of reserves > 0

- target `B += fireSpeedCap` (Wumpa bonus limited so `fireSpeedCap + bonus ≤ 6144`), section 3.2;
- engine accel = `Accel_Reserves` 1152 (vs 448-544), section 3.4;
- Cross is auto-held, section 3.1;
- no turn speed penalty, section 3.3;
- reserves drain by `ms` per frame (`VehPhysProc.c:251-260`);
- Square zeroes them (`:954-963`); a SOLID wall scrub zeroes them (`COLL.c:2542-2546`).

When reserves run out, `B` drops back and forward friction (120/frame) bleeds the excess.

### 7.3 Sources

| Source | Reserves | Fire level → cap | Cite |
|---|---|---|---|
| Powerslide boost 1/2/3 | 1920-4·meterLeft | 0/64/128 → 2048/2560/3072 | `VehPhysProc.c:2486-2501` |
| Hang time | 960 | 0/128/256 → 2048/3072/4096 | `UI_Meter.c:88-102` |
| Turbo pad | 960 (frozen while on pad) | 256 → 4096; engine accel 8000 | `VehPhysForce.c:1003-1028`, `VehPhysGeneral.c:1189-1239` |
| Super turbo pad | 120 (frozen) | 0x800 → 18432 | same |
| Turbo item | 2400 (FREEZE type) | 128 → 3072; **10 Wumpa (juiced): 256 → 4096** | `VehPickupItem.c:105-108`, `611-617` |
| Start-line rev boost | 960 | 0x20 → 2304 (small) or 0x80 → 3072 (big) | `VehStuckProc.c:11-13`, `967-983` |
| Super Engine item/cheat | 120 per frame while gassing | 128 (juiced 256) | `VehPhysProc.c:1286-1308` |

### 7.4 Wumpa

`numWumpas` (capped at 9 for speed) gives up to +269 top speed for all classes (3.2). 10 Wumpa ("juiced") only changes item strength (turbo item fire 128 → 256, Super Engine fire 128 → 256) and SFX.

The turbo fire thread (`VehTurbo.c`) is purely visual/audio (fire size = `5 + fireLevel>>6`, max 8, `VehFire.c:322-328`).

---

## 8. Terrain interaction

### 8.1 Ground normal → orientation

- **Physics frame**: `matrixMovingDir = RotAxisAngle(AxisAngle1, angle)` (`VehPhysGeneral.c:932`, `VehPhysProc.c:2442`). AxisAngle1 is the **raw normal** of the ground triangle touched this frame. It is set by the sweep (`COLL.c:2395-2398`) or the probe (`COLL.c:1446-1450`), and reset to up every frame (`VehPhysForce.c:968-973`).
- **`RotAxisAngle(n, a)`** (`VehPhysForce.c:1799-2044`):
  - column Y = n;
  - column Z = Rodrigues minimal rotation (up → n) applied to `(sin a, 0, cos a)`. Explicitly: `Z.y = -(sin a·n.x + cos a·n.z)`, and the X/Z parts are `s·n.y + ((s - s·n.y)·n.z² + (c - c·n.y)·(-n.x·n.z))/(n.x²+n.z²)` (and symmetric);
  - column X = `n × Z` (GTE OP). If `n.x = n.z = 0`, it is a plain yaw.
- **Visual frame**:
  - `AxisAngle2 = normalize((w·AxisAngle2 + (8-w)·normalVecUP) / 8)`, with w = 6 on ground and 7 in air (`normalVecUP` is up in air, `COLL.c:1546-1577`);
  - `matrixFacingDir = RotAxisAngle(AxisAngle2, rotCurr.y)` (`VehPhysForce.c:1775`);
  - roll `rotCurr.z` from AxisAngle2 (`COLL.c:1580-1585`).
- **Slope effects**: gravity is projected into the local frame (section 6.4):
  - uphill gravity is ignored while the engine accelerates;
  - downhill adds up to +5120 over target;
  - lateral gravity is limited to 2560.
- **Velocity on the ground**: speed is a 3D magnitude, re-expanded along `(axisRotationX, axisRotationY)` each frame. The ground scrub removes the into-surface component, so the velocity follows the terrain.

### 8.2 Terrain table (`MetaDataTerrain`, `zGlobal_DATA.c:6976-7116`; struct `regionsEXE.h:146-196`)

The fields used by physics are 256-based:

| Field | Meaning |
|---|---|
| `speedMultiplier` | top-speed multiplier |
| `slowUntilSpeed` | **engine-accel multiplier** (the name is wrong) |
| `counterSteerRatio` | counter-steer nudge |
| `turnLeanScale` | how much the velocity heading turns with the kart |
| `groundFrictionScale` | ground friction multiplier |
| `turnAngleScale` | visual turn-angle rate |
| `turnResponseScale` | steering response |

Flags (`regionsEXE.h:95-106`): `0x4` = accel while reverse-sliding, `0x10` = skip turn assist (wobble), `0x80` = mud physics, `0x100` = sideslip friction (`0x1`, `0x8`, `0x20`, `0x40` are visual/audio).

| Terrain | flags | speedMul | accelMul | counterSteer | lean | groundFric | turnAngle | turnResp |
|---|---|---|---|---|---|---|---|---|
| asphalt / stone / metal / river/steam/ocean asphalt | 0x40 | 256 | 256 | 256 | 256 | 256 | 256 | 256 |
| dirt, grass | 0 | **140** | 180 | 256 | 256 | 256 | 256 | 256 |
| slowgrass, slowdirt | 0 | **100** | 180 | 256 | 256 | 256 | 256 | 256 |
| track (offroad edge) | 0x21 | 140 | 180 | 256 | 256 | 256 | 256 | 256 |
| wood | 0x21 | 256 | 256 | 256 | 256 | 256 | 256 | 256 |
| water | 0 | 180 | 200 | 256 | 256 | 256 | 256 | 256 |
| fastwater, hardpack | 0 | 256 | 256 | 256 | 256 | 256 | 256 | 256 |
| **ice** | 0x1C | 256 | 100 | **0** | **0** | **32** | 128 | **32** |
| icy road | 0x40 | 256 | 256 | 256 | 256 | 128 | 256 | 256 |
| snow | 0 | 140 | 180 | 256 | 256 | 128 | 256 | 256 |
| **mud** | 0x80 | 140 | 180 | 256 | 256 | **512** | 256 | 256 |
| sideslip | 0x140 | 256 | 256 | 256 | 256 | 256 | 256 | 256 |
| none (airborne) | 0 | 256 | 256 | 0 | 256 | 256 | 256 | 256 |

Extra terrain-specific code:
- Mud: sinks to y = -4096 (`VehPhysForce.c:948-959`); terminal fall speed 256; minimum frictions (3.6).
- Leaving ice gives a 320 ms grip burst (`COLL.c:1478-1481`).
- `terrainMeta2` (speed multiplier) persists up to 128 lu above the ground (`COLL.c:1458-1461`).

For WoW, map ground material (or liquid/texture) to these presets; start with asphalt everywhere.

---

## 9. Collision: what the port must provide, and wall response

### 9.1 Movement sweep (`COLL_MOVED_PlayerSearch`, `COLL.c:2246-2427`)

The collider is a **sphere of radius 25 lu** centered at `pos + matrixFacingDir*(0,25,0)`.

Up to 15 iterations:
```
step = vel * remainingFraction (start 1.0)
sweep the sphere from current to current+step against the triangles
pos += step * hitFraction
on hit:
    ground-flagged triangle: currBlockTouching = tri, AxisAngle1 = normalVecUP = tri.normal,
                             GROUNDED, scrub = ROAD
    otherwise:               scrub = SOLID
    ScrubImpact(scrub, vel)
    remainingFraction -= remainingFraction*hitFraction
    stop if remainingFraction < 100/4096
```
In CTR the ground/wall classification is the per-quad `QUADBLOCK_FLAG_GROUND`. **For WoW, classify by slope** (suggestion: `n.y ≥ cos 50°` = ground; tune). Repeated hits on the same triangle in one frame add `scrubDepth += 256` (cap ~1024) to push out (`COLL.c:2120-2166`).

### 9.2 `COLL_MOVED_ScrubImpact` (`COLL.c:2496-2662`)

Scrub table: `zGlobal_DATA.c:6512-6534`:
- SOLID `{flags 3, speedLimit 0xD00, impactAngle 700}`;
- ROAD `{flags 0xD, 0x3E800, 0}`.

```
dot = (v·n)/4096 - scrubDepth
if dot < 0:
   if !(flags & 4): flags |= DRIVING_AGAINST_WALL               (SOLID only; ends drift)
   if !(flags & 8): reserves = 0, turbo_outsideTimer = 0         (SOLID only)
   if speedLimit qualifies: wallRubTimer = 240, wallRubSpeedLimit = 3328, posWallColl = hit
   if flags & 1: v -= dot * n / 4096                             (remove into-surface velocity, inelastic)
       landing on ground from air: re-project the horizontal velocity along the surface at |speedApprox| and push 0.5*n out (:2583-2604)
       if flags & 2 (SOLID) and dot < -5119 and |impact_xz| > 5120 and the hit is within ~28° of head-on
         (|dot|/|v| > sin(700/4096·360°) = 0.879):  -> SlamWall (speed = velocity = reserves = 0, KS_CRASHING)
```

Wall-rub consequences:
- `baseSpeed` is clamped to ±3328 for 240 ms (3.3);
- steer strength is 48 (4.1);
- the counter-steer nudge is off;
- the drift ends;
- the visual turn-lerp freezes on the first frame.

### 9.3 Ground probe (`COLL_FIXED_PlayerSearch`, `COLL.c:1341-1674`)

- Cast a vertical segment from `pos.y + 128` down to `pos.y - 256` (lu) at the kart's XZ, keeping the ground hit height `quadBlockHeight`.
- If a ground triangle is hit and `pos.y ≤ quadBlockHeight + 16 lu`, the kart is grounded: `TOUCH_GROUND`, terrain set, coyote = 160.
- There is **no position snapping**: gravity plus the sweep keep the kart on the surface.

### 9.4 Other pieces

- Surface pushback (`VehPhysForce.c:1074-1183`): if the origin ended up behind the last hit plane, `vel += (pos - hitPos) * 64` per axis.
- Kart-kart collisions (`VehPhysCrash_AnyTwoCars`, `CollisionWeight` 256/256/300/200): out of scope.
- V-shift guard (`VehPhysForce.c:830-916`): counts sign flips of the local forward velocity within 640 ms. 5 flips freeze the kart until the player gives input again. Keep it optional; it may help on jagged WoW meshes.

---

## 10. Character classes and MetaPhys constants

Class enum: `namespace_Vehicle.h:563-578`. Character → class: `MetaDataCharacters`, `zGlobal_DATA.c:5230-5360`.

| Class | Characters (NTSC-U) |
|---|---|
| **BALANCED** (0) | Crash, Cortex, Komodo Joe, Fake Crash, N. Oxide |
| **ACCEL** (1) | Coco, N. Gin, Pinstripe |
| **SPEED** (2) | Tiny, Dingodile, Papu Papu, N. Tropy |
| **TURN** (3) | Polar, Pura, Ripper Roo, Penta Penguin |

There is no "Max" class in this table: NTSC-U Penta is TURN.

MetaPhys table: `zGlobal_DATA.c:7122-7189`. Layout `{debugStr, driverOffset, size, BAL, ACC, SPD, TRN}`. It is copied into `Driver.const_*` at spawn by `VehBirth.c:642-692` using `characterMetadata[characterIDs[driverID]].engineID`. Enum and offsets: `namespace_Vehicle.h:217-561`.

| # | Constant | BAL | ACC | SPD | TRN | Used in |
|---|---|---|---|---|---|---|
| 0x00 | Gravity | 900 | 900 | 900 | 900 | 6.4 |
| 0x01 | JumpForce | 4596 | = | = | = | 6.2 |
| 0x02 | PedalFriction_Perpendicular | 350 | = | = | = | 3.6 |
| 0x03 | PedalFriction_Forward | 120 | = | = | = | 3.6 |
| 0x04 | NoPedalFriction_Perpendicular | 425 | = | = | = | 3.6 |
| 0x05 | NoPedalFriction_Forward | 120 | = | = | = | 3.6 |
| 0x06 | BrakeFriction | 260 | = | = | = | 3.6 |
| 0x07 | DriftCurve (drift perp friction) | 120 | = | = | = | 3.6 |
| 0x08 | DriftFriction (drift fwd friction) | 120 | = | = | = | 3.6 |
| 0x09 | **Accel_ClassStat** | **480** | **544** | **448** | **512** | 3.4 |
| 0x0A | Accel_Reserves | 1152 | = | = | = | 3.4 |
| 0x0B | **Speed_ClassStat** (top speed) | **13140** | **13520** | **13900** | **12950** | 3.2 |
| 0x0C | AccelSpeed_ClassStat | 14640 | 15020 | 15400 | 14450 | Wumpa bonus only (3.2), HUD |
| 0x0D | SingleTurboSpeed | 2048 | = | = | = | 7.1 |
| 0x0E | SacredFireSpeed | 4096 | = | = | = | 7.1 |
| 0x0F | BackwardSpeed | 5120 | = | = | = | 3.2 |
| 0x10 | MaskSpeed | 3900 | = | = | = | 3.2 |
| 0x11 | DamagedSpeed | 4096 | = | = | = | 3.2 |
| 0x12 | **TurnRate** (u8) | **28** | **26** | **24** | **30** | 4.1 |
| 0x13 | BackwardTurnRate (u8) | 48 | = | = | = | 3.3 (range divisor) |
| 0x14 | **TurnDecreaseRate** | **4080** | **9080** | **5666** | **7252** | 3.3 |
| 0x15 | **TurnInputDelay** | **5000** | **4500** | **4000** | **5500** | 4.2 |
| 0x16 | PreTurbo | 1000 | 750 | 500 | 1250 | unused |
| 0x17 | TerminalVelocity | 20480 | = | = | = | 6.4 |
| 0x18 | TerrainFrictionBoost | 256 | = | = | = | 3.6 |
| 0x19-0x1B | SteerAccel Stage4First / Stage2First / Stage2Len (s8) | 64 / 4 / 8 | = | = | = | 4.4 |
| 0x1C-0x1D | SteerAccel Stage1 Max / Min | 3072 / 0 | = | = | = | 4.4 |
| 0x1E-0x1F | SteerAccelTurnVelScale / Limit | 128 / 0 | = | = | = | 4.4 |
| 0x20-0x21 | ModelRotVelMax / Min | 350 / 450 | = | = | = | 4.6 |
| 0x22-0x25 | ModelTurn CounterSteerStrength / Return / NegReturn / VelLerp (u8) | 30 / 20 / 60 / 10 | = | = | = | 4.6 |
| 0x26-0x27 | TurnResistMin / Max (u8) | 32 / 224 | = | = | = | 4.6 |
| 0x28 | SteerVel_DriftSwitchWay (s8) | -10 | = | = | = | 5.2 |
| 0x29 | SteerVel_DriftStandard (s8) | 22 | = | = | = | 5.2 |
| 0x2A | **DriftTurnBase** (s8) | **14** | **10** | **5** | **18** | 5.1 |
| 0x2B | DriftTurnStartupScale | 100 | = | = | = | 5.2 |
| 0x2C | DriftTurnRampFrames | 60 | = | = | = | 5.2 |
| 0x2D | Drifting_FramesTillSpinout | 60 | = | = | = | 5.4 |
| 0x2E-0x2F | DriftSpinRateAccel / Decel | 3000 / 500 | = | = | = | 5.2 |
| 0x30-0x31 | DriftCameraSpinRate / CameraLerpStep | 384 / 16 | = | = | = | visual |
| 0x32 | DriftReleaseTurnAssistFrames | 15 | = | = | = | 5.7 |
| 0x33-0x34 | MetaPhys33/34 | 8192 / 8192 | = | = | = | unused |
| 0x35-0x37 | DriftTurn SameDirAngle / OppositeDirAngle / AngleScale | 300 / -128 / 768 | = | = | = | 5.2 (visual drift angle) |
| 0x38 | TurboMaxRoom | 30 | = | = | = | 5.3 |
| 0x39 | TurboLowRoomWarning | 15 | = | = | = | 5.3 |
| 0x3A | TurboFullBarReserveGain | 60 | = | = | = | 5.3 |
| 0x3B | DriftBoostDurationFrames | 15 | = | = | = | 5.2 (i) |
| 0x3C | DriftBoostAxisKickRate | 36 | = | = | = | 5.2 (i) |
| 0x3D | CollisionWeight | 256 | 256 | 300 | 200 | kart-kart |
| 0x3E | SlopeForwardSpeedBonus | 5120 | = | = | = | 6.4 |
| 0x3F | SideSpeedClamp | 2560 | = | = | = | 6.4 |
| 0x40 | prototypeKey | 15 | = | = | = | unused |

(`=` means the same value for all four classes.)

**Derived per-class figures** (ms = 32, asphalt, 0 Wumpa):

| | BAL | ACC | SPD | TRN |
|---|---|---|---|---|
| Top speed (lu/s) | 1540 | 1584 | 1629 | 1518 |
| Net accel at full gas (/frame) | 360 | 424 | 328 | 392 |
| 0 → top (frames, ignoring the steer ramp) | ≈ 37 | ≈ 32 | ≈ 42 | ≈ 33 |
| Max steer yaw (°/s) | 74 | 69 | 63 | 79 |
| Top-speed loss at full steer | -2380 (18%) | -4918 (36%) | -2833 (20%) | -4532 (35%) |
| Drift neutral / max yaw (°/frame) | 1.23 / 3.16 | 0.88 / 2.81 | 0.44 / 2.37 | 1.58 / 3.52 |
| Drift entry min speed | 6570 | 6760 | 6950 | 6475 |

---

## 11. Core per-frame pseudo-code (Rust-flavoured, ms = 32)

```rust
const MS: i32 = 32;
fn shr(x: i32, s: u32) -> i32 { x >> s }               // arithmetic
fn map(v,omin,omax,nmin,nmax) -> i32 { /* VehCalc_MapToRange, see section 0 */ }
fn approach(v, step, target) -> i32 { /* InterpBySpeed */ }

fn tick(d: &mut Driver, inp: &Input, world: &impl Collider) {
    // ---- 0/1: state entry + transitions (use LAST frame's flags) ----
    match d.state {
        Normal   => if d.flags.started_touch_ground
                     && d.simp_turn.abs() > (d.c.turn_rate as i32) / 2
                     && inp.held(d.drift_button) && !d.flags.accel_prevention
                     && d.speed_approx >= d.c.speed_class / 2 { powerslide_init(d) },
        Drifting => powerslide_update(d, inp),          // 5.3 + exit/spin checks (2.3)
        _ => {}
    }

    // ---- 2: PHYS_LINEAR (VehPhysProc_Driving_PhysLinear) ----
    d.flags_prev = d.flags;  d.pos_prev = d.pos;  d.turn_angle_prev = d.turn_angle;
    decrement_timers(d, MS);       // reserves, turbo_outside, wall_rub, jump_forced, jump_cooldown, ... (ms); jump_ten_buffer (frames)
    d.flags &= CARRY_MASK;
    jump_button_logic(d, inp);     // 5.1: tap -> ten_buffer=10, drift_button; held -> JUMP_BUTTON_HELD
    if inp.square && !on_turbo_pad(d) { d.reserves = 0; }
    let cross = inp.cross || d.reserves != 0;
    let b = base_speed(d);                                         // 3.2
    let (gas, flags) = throttle_table(inp, cross, b, d);           // 3.2 table
    d.fire_speed = gas;
    let (gas, b) = if !flags.brake_with_accel && !d.flags.mask {
        let m = d.terrain2.speed_mul; (gas*m>>8, b*m>>8) } else { (gas, b) };
    d.terrain_scaled_base = b;  d.base_speed = gas;
    d.simp_turn = steer_command(d, inp);                           // 4.1 (+ STEER_LEFT / frames-steering logic)
    if d.state == Drifting { d.flags.back_skid = true; d.flags.front_skid = true; }

    // ---- 4: PHYS_ANGULAR ----
    if d.state == Drifting { powerslide_phys_angular(d); }          // 5.2
    else {                                                           // 4.2 - 4.6
        let mut rot = turn_rate_response(d);
        rot += post_drift_assist(d);
        visual_turn_angle_update(d, rot);                           // 4.6 (turn_angle, lerp vel, low-speed transfer)
        rot = steer_ramp(d, rot);                                   // 4.4
        d.amp_turn = rot;
        d.angle = (d.angle + (rot*MS >> 13)) & 0xFFF;
        let lean = if d.flags.accel_prevention || d.accel_tap_count >= 7 { rot*10>>8 }
                   else { rot*d.terrain1.turn_lean_scale>>8 };
        d.axis_rot_x = (d.axis_rot_x + (lean*MS >> 13)) & 0xFFF;
        d.mat_moving = rot_axis_angle(d.axis_angle1, d.angle);
        counter_steer(d);                                           // sets d.accel (one-frame nudge)
    }

    // ---- 5: APPLY_FORCES (VehPhysForce_OnApplyForces + OnGravity) ----
    d.speed = d.speed.min(25600);
    d.origin_to_center = d.mat_facing * (0, 25, 0);
    d.vel = speed_to_vec(d.speed, d.axis_rot_x, d.axis_rot_y);
    let mut l = d.mat_moving.transpose() * d.vel;                  // local (x=left, y=up, z=fwd)
    let l0 = l;
    let mut g = d.mat_moving.transpose() * (0, -(d.c.gravity*MS>>5), 0);
    if (g.z < 0 && d.fwd_accel_impulse > 0) || (g.z > 0 && d.fwd_accel_impulse < 0) { g.z = 0; }
    if d.flags.accel_prevention || opposite_sign(d.base_speed, d.speed_approx) { g.x = 0; g.z = 0; }
    l += g;  clamp_gravity_push(&mut l, l0, d);                    // 6.4 limits
    if d.flags_prev.touch_ground || d.state == Blasted || slow_terrain_overspeed(d) {
        apply_friction(&mut l, d);                                  // 3.6
    }
    d.vel = d.mat_moving * l;
    update_forward_dir_and_vshift(d, l0.z, l.z);
    d.vel += d.accel;                                               // counter-steer nudge
    d.axis_angle1 = UP; d.normal_up = UP; d.curr_block = None; d.coll_flags = 0;

    // ---- 6: COLL_MOVED: swept sphere + slide, scrub impacts (9.1/9.2) ----
    world.sweep_and_slide(d);          // moves d.pos by d.vel; may set ground normal/GROUNDED, wall rub, slam

    // ---- 7: COLLIDE_DRIVERS ----
    d.vel -= d.accel;
    turbo_pads(d);                     // optional, 7.3
    surface_pushback(d);               // 9.4

    // ---- 8: COLL_FIXED: ground probe (9.3) ----
    world.ground_probe(d);             // TOUCH_GROUND/STARTED_TOUCH_GROUND, terrain, coyote, airtime, AIRBORNE, normals

    // ---- 9: JUMP_FRICTION (VehPhysGeneral_JumpAndFriction) ----
    if d.state != Drifting && !d.flags.mask && d.reserves == 0 { turn_speed_penalty(d); }   // 3.3
    if d.wall_rub_timer != 0 { d.base_speed = d.base_speed.clamp(-d.wall_rub_limit, d.wall_rub_limit); }
    let mut mv = d.vel;
    let mut loss = 0;
    if d.flags.touch_ground {
        let acc = engine_accel(d) * MS >> 5;                       // 3.4
        let f = d.mat_moving * (0, 0, acc);
        if d.base_speed >= 0 { mv += f; d.fwd_accel_impulse = acc; } else { mv -= f; d.fwd_accel_impulse = -acc; }
        loss = (isqrt(mv.len_sq()) - d.base_speed.abs()).clamp(0, acc);
    }
    if d.flags.touch_ground && d.jump_forced_ms != 0 { process_jump(d, &mut mv); }
    else if !try_spring_or_forced_jump(d, &mut mv)
         && d.coyote_ms != 0 && d.jump_ten_buffer != 0 && d.jump_cooldown_ms == 0 {
        d.jump_forced_ms = 160; d.jump_initial_vy = d.c.jump_force; process_jump(d, &mut mv);  // 6.2/6.3
    }
    vec_to_speed(d, mv);               // speed=|mv|, axis_rot_x=atan2(x,z), axis_rot_y, speed_approx, jump_height
    d.speed = (d.speed - loss).max(0);

    // ---- 10: visual ----
    d.mat_facing = rot_axis_angle(d.axis_angle2, d.rot_curr_y);

    // ---- after physics: hang-time boost (6.6) ----
    jump_meter_update(d);
}
```

`powerslide_update(d, inp)`, run in UPDATE (and installed on the frame after entry):

```rust
fn powerslide_update(d: &mut Driver, inp: &Input) {
    if inp.tapped(L1) || inp.tapped(R1) {
        d.num_frames_drifting = 0;
        let m = d.turbo_meter;
        if m != 0 {
            if m < (d.c.turbo_low_room_warning as i32) << 5 {
                let gain = map(m, 0, 480, (d.c.turbo_full_bar_gain as i32) << 5, 0);
                vehfire_increment(d, gain, POWER_SLIDE_HANG_TIME, (d.boosts_success as i32) << 6);
                d.boosts_success += 1;
                d.drift_boost_ms = (d.c.drift_boost_frames as i32) << 5;
            } else {
                d.failed_boost_exhaust = 8;
            }
            d.turbo_meter = 0;
            d.boosts_attempted += 1;
        }
    } else if d.turbo_meter != 0 {
        d.turbo_meter = (d.turbo_meter - MS).max(0);
        if d.turbo_meter == 0 { d.boosts_attempted += 3; }
    } else if d.boosts_attempted < 3 {
        d.turbo_meter = (d.c.turbo_max_room as i32) << 5;
    }
    let lim = d.c.frames_till_spinout as i32;
    if d.num_frames_drifting > lim || (d.speed_approx < 0 && !on_turbo_pad(d)) { spin_init(d); }
    else if d.num_frames_drifting < -lim || d.speed < d.c.speed_class / 2
         || d.flags.against_wall || d.flags.accel_prevention || d.flags.brake_with_accel
         || !inp.held(d.drift_button) {
        d.prev_mult_drift = d.mult_drift;
        d.time_until_drift_spinout = (d.c.drift_release_assist_frames as i32) << 5;
        driving_init(d);   // turbo_meter = 0, state Normal
    }
}
```

### Implementation notes

- Keep all state in the integer units above (i32 math with s16 truncation where the original stores s16: `speed`, `angle`, `baseSpeed`, `reserves` ...). Bit-exact behaviour needs CTR's trig table (`data.trigApprox`) and `ratan2`. A float port with the same constants will feel the same but will drift numerically.
- The visual layer (`turnAngleCurr`, wobble, `rotCurr.w`, squash) can be simplified. Two pieces must stay because they feed physics: the counter-steer nudge (4.6) and the drift `turnAngleCurr` sign used by the boost kick (5.2 i).

---

## 12. Ambiguities and uncertainties

1. **lu → meters**: not defined by the code (section 0). The 2.5 cm/lu figure is an estimate.
2. **Drift visual angle maps** (`VehPhysProc.c:2195`, `2203-2239`). `MapToRange(|dti|, 0, DriftTurnBase(+..), 0, 768)` compares a value in the thousands against an unshifted 5..18, so it always returns 768. The counter-steer maps use a negative `oldMax` (`SwitchWay<<8`), so they saturate to the end value immediately. Implement as written; it looks like an original-dev bug but it is what retail does.
3. **`slowUntilSpeed`** is named like a speed but is used only as an acceleration multiplier (`VehPhysGeneral.c:1246-1253`).
4. **Reverse controls**: per code, Down without Cross reverses at full `BackwardSpeed`. Square alone only brakes. Square+Down reverses at 3/4 (`VehPhysProc.c:1085-1217`).
5. **Gravity-Z suppression** while `forwardAccelImpulse` has the opposite sign (`VehPhysForce.c:298-301`) means uphill slopes do not slow an accelerating kart. This is surprising but explicit in the code.
6. **Drift button**: boosts need any L1/R1 *tap*. Because the drift button must stay held, in practice boosts come from the other shoulder button (`VehPhysProc.c:2476`, `2564-2567`).
7. **Pressing timing**: an early press (meter not yet red) fails. Not pressing before the meter fills cancels further boosts. There is no "late press backfire"; the failure exhaust is for early presses.
8. **Collision differences**: CTR classifies ground/wall per quad flag. WoW needs a slope threshold. Turbo pads, kill planes, mask-grab respawn, the anti-gravity and low-gravity quads, and the V-shift guard are level-feature dependent and optional.
9. **The UPDATE stage runs before PHYS_LINEAR**, so drift entry and boost presses are evaluated against last frame's derived flags but this frame's `buttonsTapped`. Keep this order to reproduce timing windows exactly.

---

## 13. Hit reactions and the self-applied items

The victim's side of the weapons in `docs/ctr-weapons-spec.md` (sections 3 and 4), as `ctr-kart`
ports it. Paths are under the decompilation's `game/`. Timers are CTR-ms (32 a frame) unless
marked frames.

### 13.1 The gate (`Vehicle/VehPickState.c:48-114`)

`RB_Hazard_HurtDriver` → `VehPickState_NewState(victim, type, …)`, in order:

1. A mask (`ACTION_MASK_WEAPON`, set while the mask's thread lives, fade included,
   `VehPhysProc.c:719-764`) or `invincibleTimer != 0`: ignored, the victim laughs, returns 0.
2. A shield: it pops, `invincibleTimer = 0x2A0` (672 = 21 frames), returns 0. One hit absorbed.
3. Type 0 (a TNT asking to sit on the head): returns 1, nothing else.
4. Otherwise the type's reaction below, then for every one: `reserves = 0`,
   `turbo_outsideTimer = 0` (the boost is cancelled). The reaction's INIT runs at the start of
   the **next** frame (`funcPtrs[DRIVER_FUNC_INIT]`, the first driver slot).

| Type | `NoInputTimer` | Notes | Cite |
|---|---|---|---|
| 1 spin-out | `0x3C0` (30 frames) | already `KS_SPINNING`: only the timer is reset | `VehPickState.c:238-246` |
| 2 blasted | `0x960` (75 frames) | ignored when `KS_BLASTED` or the tumble INIT is pending; clears `squishTimer` | `:151-184` |
| 3 squished | `0xF0` (7.5 frames) | `squishTimer = 0xF00` (120 frames); SpinFirst re-inits even mid-spin | `:227-236` |

Retail's common tail also sets `kartState = KS_NORMAL`, so a spin re-hit mid-spin leaves the spin
functions running in a "normal" state (no friction halving, fire allowed). The port keeps the
spinning state: a quirk, not a mechanism.

### 13.2 Spin-out (`Vehicle/VehPhysProc.c:2766-3150`)

`KS_SPINNING` through three phases. In all of them `baseSpeed = fireSpeed = 0`; steering is
ignored (`numFramesSpentSteering = 10000`); the drawn model is `angle + turnAngleCurr`.

| Phase | PHYS_LINEAR | PHYS_ANGULAR | Leaves when (UPDATE) |
|---|---|---|---|
| SpinFirst | `NoInputTimer −= ms`; `ACCEL_PREVENTION` (friction `BrakeFriction/2` = 130) | `rotationSpinRate −= rate>>3`; `ampTurnState = rate`; `turnAngleCurr += ±300` wrapped to ±2048 (sign of `ampTurnState` at init) | `NoInputTimer == 0` or `\|speedApprox\| ≤ 0x2FF` |
| SpinLast | `ACCEL_PREVENTION` | same decay; keeps whirling until the angle comes round within 400 of 0, then the rate is `−angle·4>>3`, at least 32, clamped at 0 | `\|turnAngleCurr\| < 16` |
| SpinStop | no brake (coasting friction) | `turnAngleCurr` eases to 0 by 128 a frame; yaw keeps the last `ampTurnState` | the steering animation is home (`SpinStop_Animate`), then Driving_Init |

A drift's spin-out enters SpinFirst directly with 30 frames, or 60 (`0x780`) when it rolled
backwards (`:2542-2552`). SpinFirst_Init costs one Wumpa (`:2868-2871`), the caller's to apply.

### 13.3 Blasted (`Vehicle/VehStuckProc.c:1349-1504`)

`KS_BLASTED`. Each frame: `NoInputTimer −= ms`; `baseSpeed = 0`; `ACCEL_PREVENTION`;
`jump_ForcedMS = 0x60`; `jump_InitialVelY = 6000 + 2·NoInputTimer` (10800 at the start, 6000 at
the end; a hop is 4596). JUMP_FRICTION re-runs PROCESS_JUMP on every grounded frame while
`jump_ForcedMS != 0` (`VehPhysGeneral.c:1350-1365`), so the kart bounces. Friction is
`BrakeFriction·3/4` = 195 and applies in the air too (`VehPhysForce.c:429`, `455-468`). The
angular stage lets `rotationSpinRate` and the model's turn die away by an eighth a frame. UPDATE
returns to Driving the frame after `NoInputTimer` reaches 0: 75 frames. VehFire refuses boosts
while blasted. It costs three Wumpa (`:1466-1469`), the caller's to apply.

**The tumble's look** is procedural: `bakedGteMath[BLASTED]`, 27 entries built by
`Vector_BakeMatrixTable` (`Vector.c:155-232`): entry `i` rolls `i·0x2000/27` (two turns about the
nose over the cycle), pitches `−sin(i·0x3000/27)/7`, scales X by `1 + sin(roll)/4` and Z by
`1 + 6·sin(roll)/40`, and is offset so the kart turns about a point `0x2000` (32 lu) above its
origin. The frame shown is `(NoInputTimer >> 5) % 27`, run backwards on `MixRNG & 4`
(`VehStuckProc.c:1433-1451`). The port composes `ConvertRotToMatrix`'s X then Z rotation on the
facing frame; the handedness of the roll is not checked against a capture.

### 13.4 Squished, the clock, the speed cuts (`VehPhysGeneral.c:2082-2105`)

`subtract`: TNT on the head `DamagedSpeed>>1` = 2048; squish (or burn, rain-cloud slow)
`DamagedSpeed` = 4096; under the clock at least `DamagedSpeed·(20 − rank)>>4` (5120 for 1st down to
3328 for 8th). `B = min(B − subtract, 0x6400)`. A squished kart is drawn with `scale.y = 0`, lifted
19 lu up its smoothed normal (`VehPhysForce.c:1453-1458`, `1737-1740`). The clock
(`VehPickupItem.c:855-908`) is a type-1 hit; if it takes, `clockReceive = 0x1E00` (juiced
`0x2D00`). `squishTimer` and `clockReceive` count down in PHYS_LINEAR (`VehPhysProc.c:326-376`).

### 13.5 TNT on the head (`231/RB_TNT.c`, `231/RB_GenericMine.c:211-301`)

Touching a TNT: with a TNT already on the head, or squished, the kart is blasted (type 2) and the
old TNT removed. Otherwise a type-0 hurt; if it passes, the TNT is thrown on: height 0, vy 48 lu a
frame, gravity 4, terminal −96, its first step at once. It sits once falling below the head
(Crash's 32 lu, the table at `RB_TNT.c:78`): about 25 frames. Then `jumpsRemaining =
8 − MixRNG % 2`; each frame a hop starts (`ACTION_JUMP_STARTED`), it decrements, and a hop at 0
throws it off: **8 or 9 hops**. `numFramesOnHead` counts to 90; on the next frame it goes off and
blasts the carrier (`:194-239`). Spinning, crashing or blasted under it sets it off at once
(`:122-159`). While it is on (and in flight): top speed −2048 and no firing (`VehPhysProc.c:809`).
The port counts the player's own hops only (forced re-jumps never start one); `MixRNG_Scramble` is
the kart's own LCG (`MixRNG.c:3-10`), seeded by the caller.

### 13.6 Mask, shield, turbo (the user's own kart)

| Item | Effect | Cite |
|---|---|---|
| Mask | `duration` 7680 / juiced 11520 counts down; at 0 a 16-frame fade (0 → 0x200) during which the thread, and so `ACTION_MASK_WEAPON`, lives on. Another mask restarts it. Invincible, `+MaskSpeed` (3900), no turn or terrain penalties. | `VehPickupItem.c:193-351`, `231/RB_MaskShieldCloud.c:19-81`, `180-202` |
| Shield | green: `duration = 0x2D00`, −32 a frame, removed the frame after it reaches 0 (361 frames), alpha fading over the last 1920; blue (juiced): no limit. One hit absorbed (13.1). ○ fires it whenever the kart is NORMAL or DRIFTING with no TNT, even under the clock. | `VehPickupItem.c:910-988`, `RB_MaskShieldCloud.c:374-463`, `VehPhysProc.c:822-829` |
| Turbo | `VehFire_Increment(2400, FREEZE \| TURBO_ITEM, 0x80 / juiced 0x100)`: 75 frames, cap +3072 / +4096 | `VehPickupItem.c:605-619` |

○ fires an item only when `kartState` is NORMAL, DRIFTING or ANTIVSHIFT, with no TNT on the head
and no `clockReceive` (`VehPhysProc.c:804-861`).

### 13.7 Not settled

- SpinStop's length is the drive animation's frame count (model data); the port uses 8 frames
  once the model faces forward.
- The TNT's landing also waits for its shrink to 0x800 (8 frames; always done by the ~25-frame
  throw) and uses the character's own head height; the port uses Crash's.
- The squish's recovery bounce (`bakedGteMath[SQUISH_RECOVER]`, `zGlobal_DATA.c:5673-5795`)
  plays its entries 1 to 14 as the squish wears off (`VehPhysForce.c:1740-1750`, `1335-1343`);
  its sound (OtherFX 0x5B) is not among the extracted sounds, so it is silent.
- The squash and stretch (`VehPhysForce.c:1643-1772`) leaves out the clock's and rain cloud's
  `hazardTimer` hold and the TNT's shrink on landing.
- In battle, leaving the tumble gives `invincibleTimer = 2880`; races only here. Over 0x2A0 the
  kart flickers on odd frames (`RB_Player_ToggleFlicker`, `231/RB_Player.c:231-257`); the port
  draws that, but in a race nothing gives more than the shield pop's 0x2A0, so it never shows.
