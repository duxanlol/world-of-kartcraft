use super::*;

const CELL: f32 = 256.0;

fn quad(a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) -> [[[f32; 3]; 3]; 2] {
    [[a, b, c], [a, c, d]]
}

/// Flat ground at y = 0 over x, z in [−s, s], in 2000 lu tiles (a face may span at most 4096
/// grid cells).
fn flat(s: f32) -> Collision {
    let n = (2.0 * s / 2000.0).ceil() as i32;
    let step = 2.0 * s / n as f32;
    let mut tris = Vec::new();
    for i in 0..n {
        for j in 0..n {
            let (x0, z0) = (-s + i as f32 * step, -s + j as f32 * step);
            let (x1, z1) = (x0 + step, z0 + step);
            tris.extend(quad(
                [x0, 0.0, z0],
                [x0, 0.0, z1],
                [x1, 0.0, z1],
                [x1, 0.0, z0],
            ));
        }
    }
    Collision::new(tris, CELL)
}

fn kart(id: KartId, x: f32, z: f32) -> KartView {
    KartView {
        id,
        position: Vec3::new(x, 0.0, z),
        rank: id as usize,
        ..KartView::default()
    }
}

fn hits(out: &[Outcome]) -> Vec<(KartId, HitKind)> {
    out.iter()
        .filter_map(|o| match o {
            Outcome::Hit { kart, kind } => Some((*kart, *kind)),
            _ => None,
        })
        .collect()
}

fn exploded(out: &[Outcome]) -> Option<(Vec3, f32)> {
    out.iter().find_map(|o| match o {
        Outcome::Exploded { at, radius } => Some((*at, *radius)),
        _ => None,
    })
}

#[test]
fn turn_takes_the_short_way() {
    assert_eq!(turn(0, 100, 64), 64);
    assert_eq!(turn(0, 4000, 64), 4032);
    assert_eq!(turn(10, 40, 64), 40);
    assert_eq!(yaw_to(Vec3::X), 1024);
    assert_eq!(yaw_to(Vec3::Z), 0);
}

#[test]
fn a_bomb_rolls_96_a_frame_on_the_ground() {
    let world = flat(20000.0);
    let mut bomb = Bomb::new(1, Vec3::ZERO, 0, false, false, None);
    for _ in 0..10 {
        assert!(bomb.step(&world, &[]).is_empty());
    }
    let p = bomb.position();
    assert!((p.z - 960.0).abs() < 1e-2, "{p}");
    assert!((p.y - RIDE_HEIGHT).abs() < 1e-3);
    let mut back = Bomb::new(1, Vec3::ZERO, 0, false, true, None);
    back.step(&world, &[]);
    assert!((back.position().z + BOMB_BACK_SPEED).abs() < 1e-3);
}

/// Flat to z = 2000, a 20° ramp up to z = 4000, a plateau, and a wall at z = 5000.
fn ramp_and_wall() -> Collision {
    let s = 3000.0;
    let rise = 2000.0 * 20f32.to_radians().tan();
    let mut tris = Vec::new();
    tris.extend(quad(
        [-s, 0.0, -s],
        [-s, 0.0, 2000.0],
        [s, 0.0, 2000.0],
        [s, 0.0, -s],
    ));
    tris.extend(quad(
        [-s, 0.0, 2000.0],
        [-s, rise, 4000.0],
        [s, rise, 4000.0],
        [s, 0.0, 2000.0],
    ));
    tris.extend(quad(
        [-s, rise, 4000.0],
        [-s, rise, 6000.0],
        [s, rise, 6000.0],
        [s, rise, 4000.0],
    ));
    tris.extend(quad(
        [-s, rise, 5000.0],
        [-s, rise + 1000.0, 5000.0],
        [s, rise + 1000.0, 5000.0],
        [s, rise, 5000.0],
    ));
    Collision::new(tris, CELL)
}

#[test]
fn a_bomb_climbs_a_ramp_and_explodes_on_a_wall() {
    let world = ramp_and_wall();
    let rise = 2000.0 * 20f32.to_radians().tan();
    let mut bomb = Bomb::new(1, Vec3::new(0.0, 0.0, 100.0), 0, false, false, None);
    let mut on_ramp = false;
    for _ in 0..100 {
        let out = bomb.step(&world, &[]);
        let p = bomb.position();
        if p.z > 2200.0 && p.z < 3800.0 {
            // Riding the slope: 48 over the ramp's height there.
            let ground = (p.z - 2000.0) / 2000.0 * rise;
            assert!((p.y - ground - RIDE_HEIGHT).abs() < 1.0, "{p} vs {ground}");
            on_ramp = true;
        }
        if let Some((at, radius)) = exploded(&out) {
            assert!(on_ramp);
            assert!(at.z < 5000.0 && at.z > 5000.0 - 2.0 * BOMB_SPEED, "{at}");
            assert!((at.y - rise - RIDE_HEIGHT).abs() < 1.0);
            assert_eq!(radius, BLAST_RADIUS);
            assert!(bomb.done());
            return;
        }
    }
    panic!("never exploded: {}", bomb.position());
}

#[test]
fn a_bomb_off_the_edge_falls_then_explodes_in_the_void() {
    let world = flat(1000.0);
    let mut bomb = Bomb::new(1, Vec3::ZERO, 0, false, false, None);
    let mut frames = 0;
    while exploded(&bomb.step(&world, &[])).is_none() {
        frames += 1;
        assert!(frames < 100);
    }
    // Off the floor at z = 1000 it finds no ground within 2304 below: gone at once.
    assert!(bomb.position().z > 1000.0 && bomb.position().z < 1100.0);
}

#[test]
fn juiced_blasts_reach_further() {
    let world = flat(20000.0);
    // The owner beside the bomb, a rival 400 lu away.
    let karts = [kart(1, 0.0, 0.0), kart(2, 400.0, 0.0)];
    for (juiced, hit) in [(false, false), (true, true)] {
        let mut bomb = Bomb::new(1, Vec3::ZERO, 2048, juiced, false, None);
        bomb.detonate();
        let out = bomb.step(&world, &karts);
        let (_, radius) = exploded(&out).unwrap();
        assert_eq!(radius, if juiced { 512.0 } else { 320.0 });
        let h = hits(&out);
        assert_eq!(h.contains(&(2, HitKind::Blasted)), hit, "{juiced}");
        assert!(!h.iter().any(|(k, _)| *k == 1), "the owner is never hurt");
    }
}

#[test]
fn a_bomb_spares_its_owner_for_60_frames() {
    let world = flat(20000.0);
    // The owner keeps pace with its own bomb.
    let mut bomb = Bomb::new(1, Vec3::ZERO, 0, false, false, None);
    for f in 0..SAFETY_FRAMES {
        let me = kart(1, 0.0, (f as f32 + 1.0) * BOMB_SPEED);
        assert!(bomb.step(&world, &[me]).is_empty(), "{f}");
    }
    let me = kart(1, 0.0, (SAFETY_FRAMES as f32 + 1.0) * BOMB_SPEED);
    let out = bomb.step(&world, &[me]);
    assert!(exploded(&out).is_some());
    assert!(hits(&out).is_empty());
}

#[test]
fn a_missile_homes_on_a_moving_target() {
    let world = flat(40000.0);
    // Fired along +Z; the target is 3000 ahead and 1500 to the side, driving across at 40.
    let mut missile = Missile::new(1, Vec3::ZERO, 0, false, Some(2));
    let mut target = Vec3::new(1500.0, 0.0, 3000.0);
    for frame in 0..300 {
        target.x += 40.0;
        let karts = [kart(1, 0.0, -5000.0), kart(2, target.x, target.z)];
        let out = missile.step(&world, &karts);
        if exploded(&out).is_some() {
            assert_eq!(hits(&out), [(2, HitKind::Blasted)]);
            assert!(frame > 30, "{frame}");
            return;
        }
    }
    panic!("missed: {} vs {target}", missile.position());
}

#[test]
fn a_juiced_missile_turns_twice_as_fast() {
    let world = flat(40000.0);
    let karts = [kart(2, 5000.0, 0.0)];
    let mut slow = Missile::new(1, Vec3::ZERO, 0, false, Some(2));
    let mut fast = Missile::new(1, Vec3::ZERO, 0, true, Some(2));
    slow.step(&world, &karts);
    fast.step(&world, &karts);
    assert_eq!(slow.core.yaw, MISSILE_TURN);
    assert_eq!(fast.core.yaw, MISSILE_TURN_JUICED);
    assert!((fast.core.vel.length() - MISSILE_SPEED_JUICED).abs() < 1e-3);
}

#[test]
fn a_lost_target_flies_straight() {
    let world = flat(40000.0);
    let mut missile = Missile::new(1, Vec3::ZERO, 0, false, Some(2));
    missile.step(&world, &[kart(2, 5000.0, 5000.0)]);
    let yaw = missile.core.yaw;
    // The target leaves; the missile keeps its heading for good, even when it comes back.
    for _ in 0..10 {
        missile.step(&world, &[]);
    }
    assert_eq!(missile.target(), None);
    missile.step(&world, &[kart(2, -5000.0, 5000.0)]);
    assert_eq!(missile.core.yaw, yaw);
}

#[test]
fn a_missile_explodes_on_a_wall() {
    // Wall across z = 1000.
    let mut tris = quad(
        [-3000.0, 0.0, -3000.0],
        [-3000.0, 0.0, 3000.0],
        [3000.0, 0.0, 3000.0],
        [3000.0, 0.0, -3000.0],
    )
    .to_vec();
    tris.extend(quad(
        [-3000.0, 0.0, 1000.0],
        [-3000.0, 500.0, 1000.0],
        [3000.0, 500.0, 1000.0],
        [3000.0, 0.0, 1000.0],
    ));
    let world = Collision::new(tris, CELL);
    let mut missile = Missile::new(1, Vec3::ZERO, 0, false, None);
    for _ in 0..20 {
        if let Some((at, radius)) = exploded(&missile.step(&world, &[])) {
            assert!(at.z < 1000.0 && at.z > 1000.0 - 2.0 * MISSILE_SPEED);
            assert_eq!(radius, MISSILE_BLAST_RADIUS);
            return;
        }
    }
    panic!("went through the wall");
}

#[test]
fn a_warp_orb_blasts_the_leader_and_dies() {
    let world = flat(40000.0);
    let karts = [
        KartView {
            rank: 2,
            ..kart(1, 0.0, 0.0)
        },
        KartView {
            rank: 1,
            ..kart(3, 300.0, 3000.0)
        },
        KartView {
            rank: 0,
            ..kart(2, -2000.0, 8000.0)
        },
    ];
    let mut orb = WarpOrb::new(1, Vec3::ZERO, 0, false, Some(2));
    let mut seen = Vec::new();
    for _ in 0..400 {
        let out = orb.step(&world, &karts);
        seen.extend(hits(&out));
        if out.contains(&Outcome::Expired) {
            assert!(seen.contains(&(2, HitKind::Blasted)), "{seen:?}");
            assert!(!seen.iter().any(|(k, _)| *k == 1));
            return;
        }
    }
    panic!("never arrived: {}", orb.position());
}

#[test]
fn a_juiced_warp_orb_works_up_the_order() {
    let world = flat(40000.0);
    let karts = [
        KartView {
            rank: 3,
            ..kart(1, 0.0, 0.0)
        },
        KartView {
            rank: 2,
            ..kart(4, 2000.0, 2000.0)
        },
        KartView {
            rank: 1,
            ..kart(3, -2000.0, 5000.0)
        },
        KartView {
            rank: 0,
            ..kart(2, 0.0, 9000.0)
        },
    ];
    // Aimed at the kart just ahead.
    let mut orb = WarpOrb::new(1, Vec3::ZERO, 0, true, Some(4));
    let mut seen = Vec::new();
    for _ in 0..800 {
        let out = orb.step(&world, &karts);
        seen.extend(hits(&out).into_iter().map(|(k, _)| k));
        if out.contains(&Outcome::Expired) {
            assert_eq!(seen, [4, 3, 2]);
            return;
        }
    }
    panic!("stalled at {} after {seen:?}", orb.position());
}

#[test]
fn a_warp_orb_with_no_target_dies() {
    let world = flat(1000.0);
    let mut orb = WarpOrb::new(1, Vec3::ZERO, 0, false, None);
    assert_eq!(orb.step(&world, &[]), [Outcome::Expired]);
}

fn mine(kind: MineKind, world: &Collision) -> Mine {
    Mine::drop(1, kind, Vec3::new(0.0, 30.0, 0.0), world)
}

#[test]
fn a_dropped_mine_settles_and_spares_its_owner_briefly() {
    let world = flat(5000.0);
    let mut tnt = mine(MineKind::Tnt, &world);
    let owner = [kart(1, 0.0, 0.0)];
    for _ in 0..SHORT_SAFETY_FRAMES {
        assert!(tnt.step(&world, &owner).is_empty());
    }
    assert_eq!(tnt.position().y, 0.0);
    assert_eq!(hits(&tnt.step(&world, &owner)), [(1, HitKind::TntOnHead)]);
}

#[test]
fn tnt_lands_on_a_head() {
    let world = flat(5000.0);
    let mut tnt = mine(MineKind::Tnt, &world);
    let out = tnt.step(&world, &[kart(2, 100.0, 0.0)]);
    assert_eq!(
        out,
        [Outcome::Hit {
            kart: 2,
            kind: HitKind::TntOnHead
        }]
    );
    assert!(tnt.done());
    // 120 lu is the edge.
    let mut tnt = mine(MineKind::Tnt, &world);
    assert!(tnt.step(&world, &[kart(2, 121.0, 0.0)]).is_empty());
}

#[test]
fn tnt_on_a_carrier_or_a_masked_kart_blows_up() {
    let world = flat(5000.0);
    let carrier = KartView {
        tnt_on_head: true,
        ..kart(2, 50.0, 0.0)
    };
    let bystander = kart(3, 0.0, 250.0);
    let mut tnt = mine(MineKind::Tnt, &world);
    let out = tnt.step(&world, &[carrier, bystander]);
    assert_eq!(hits(&out), [(2, HitKind::Blasted), (3, HitKind::Blasted)]);
    assert_eq!(exploded(&out).unwrap().1, TNT_BLOWUP_RADIUS);

    let masked = KartView {
        masked: true,
        ..kart(2, 50.0, 0.0)
    };
    let mut tnt = mine(MineKind::Tnt, &world);
    let out = tnt.step(&world, &[masked]);
    assert!(exploded(&out).is_some());
    assert_eq!(hits(&out), [(2, HitKind::Blasted)], "the mask absorbs it");
}

#[test]
fn nitro_blasts_128_to_players() {
    let world = flat(5000.0);
    let mut nitro = mine(MineKind::Nitro, &world);
    let out = nitro.step(
        &world,
        &[
            kart(2, 100.0, 0.0),
            kart(3, 0.0, 120.0),
            kart(4, 0.0, 200.0),
        ],
    );
    assert_eq!(exploded(&out).unwrap().1, NITRO_PLAYER_RADIUS);
    assert_eq!(hits(&out), [(2, HitKind::Blasted), (3, HitKind::Blasted)]);
}

#[test]
fn beakers_spin_out_and_red_rains() {
    let world = flat(5000.0);
    let mut green = mine(MineKind::Beaker { red: false }, &world);
    assert!(
        green.step(&world, &[kart(2, 90.0, 0.0)]).is_empty(),
        "80 lu"
    );
    let out = green.step(&world, &[kart(2, 70.0, 0.0)]);
    assert_eq!(hits(&out), [(2, HitKind::SpinOut)]);
    assert!(!out.contains(&Outcome::RainCloud { kart: 2 }));
    let mut red = mine(MineKind::Beaker { red: true }, &world);
    let out = red.step(&world, &[kart(2, 70.0, 0.0)]);
    assert!(out.contains(&Outcome::RainCloud { kart: 2 }));
    assert_eq!(MineKind::from_shot(Item::Tnt, true), Some(MineKind::Nitro));
}

fn key(owner: KartId, shot: u16) -> ShotKey {
    ShotKey { owner, shot }
}

#[test]
fn the_field_caps_bombs_and_missiles_at_12() {
    let mut field = Field::new();
    for i in 0..12 {
        let m = Missile::new(1, Vec3::ZERO, 0, false, None);
        assert!(field.launch(key(1, i), Body::Missile(m)));
    }
    let b = Bomb::new(1, Vec3::ZERO, 0, false, false, None);
    assert!(!field.launch(key(1, 12), Body::Bomb(b)));
    // Shield bombs and orbs are outside the cap.
    assert!(field.launch(key(1, 13), Body::Bomb(Bomb::shield(1, Vec3::ZERO, 0))));
}

#[test]
fn an_eleventh_mine_sets_off_the_oldest() {
    let world = flat(50000.0);
    let mut field = Field::new();
    for i in 0..11u16 {
        let m = Mine::drop(
            1,
            MineKind::Tnt,
            Vec3::new(i as f32 * 1000.0, 0.0, 0.0),
            &world,
        );
        field.launch(key(1, i), Body::Mine(m));
    }
    // A kart beside the oldest is caught in its blowup.
    let out = field.step(&world, &[kart(2, 0.0, 300.0)]);
    assert!(out.contains(&(
        key(1, 0),
        Outcome::Hit {
            kart: 2,
            kind: HitKind::Blasted
        }
    )));
    assert_eq!(field.len(), 10);
    assert!(field.get(key(1, 0)).is_none());
}

#[test]
fn a_bomb_and_a_missile_that_meet_both_go() {
    let world = flat(50000.0);
    let mut field = Field::new();
    let bomb = Bomb::new(1, Vec3::ZERO, 0, false, false, None);
    let missile = Missile::new(2, Vec3::new(0.0, 0.0, 2000.0), 2048, false, None);
    field.launch(key(1, 1), Body::Bomb(bomb));
    field.launch(key(2, 1), Body::Missile(missile));
    for _ in 0..20 {
        let out = field.step(&world, &[]);
        if !out.is_empty() {
            assert!(out
                .iter()
                .any(|(k, o)| *k == key(1, 1) && matches!(o, Outcome::Exploded { .. })));
            assert!(out
                .iter()
                .any(|(k, o)| *k == key(2, 1) && matches!(o, Outcome::Destroyed { .. })));
            assert!(field.is_empty());
            return;
        }
    }
    panic!("they never met");
}

#[test]
fn a_tnt_dropped_by_the_target_draws_its_missile_ten_frames() {
    let world = flat(50000.0);
    let karts = [kart(1, 0.0, -3000.0), kart(2, 0.0, 6000.0)];
    let yaw_of = |field: &Field| match field.get(key(1, 1)) {
        Some(Body::Missile(m)) => (m.core.yaw, m.seeking_tnt()),
        other => panic!("{other:?}"),
    };
    let mut field = Field::new();
    let missile = Missile::new(1, Vec3::ZERO, 0, false, Some(2));
    field.launch(key(1, 1), Body::Missile(missile));
    field.step(&world, &karts);
    assert_eq!(yaw_of(&field), (0, 0), "straight at its target");
    // The target drops a TNT off to the right: the missile turns for it, 64 a frame.
    let tnt = Mine::drop(2, MineKind::Tnt, Vec3::new(5000.0, 0.0, 1000.0), &world);
    field.launch(key(2, 1), Body::Mine(tnt));
    field.step(&world, &karts);
    assert_eq!(yaw_of(&field), (MISSILE_TURN, DECOY_FRAMES - 1));
    for _ in 0..9 {
        field.step(&world, &karts);
    }
    let (yaw, left) = yaw_of(&field);
    assert_eq!((yaw, left), (10 * MISSILE_TURN, 0));
    // Then back after its target.
    field.step(&world, &karts);
    assert_eq!(yaw_of(&field).0, 9 * MISSILE_TURN);
    // A beaker sets the flag but is no TNT: the missile holds its heading and stops seeking.
    let mut field = Field::new();
    let missile = Missile::new(1, Vec3::ZERO, 256, false, Some(2));
    field.launch(key(1, 1), Body::Missile(missile));
    let beaker = Mine::drop(
        2,
        MineKind::Beaker { red: false },
        Vec3::new(3000.0, 0.0, 2500.0),
        &world,
    );
    field.launch(key(2, 1), Body::Mine(beaker));
    field.step(&world, &karts);
    assert_eq!(yaw_of(&field), (256, 0));
}

#[test]
fn a_blast_sets_off_mines_in_reach() {
    let world = flat(50000.0);
    let mut field = Field::new();
    let near = Mine::drop(3, MineKind::Tnt, Vec3::new(250.0, 0.0, 0.0), &world);
    let far = Mine::drop(3, MineKind::Tnt, Vec3::new(600.0, 0.0, 0.0), &world);
    field.launch(key(3, 1), Body::Mine(near));
    field.launch(key(3, 2), Body::Mine(far));
    let mut bomb = Bomb::new(1, Vec3::ZERO, 2048, false, false, None);
    bomb.detonate();
    field.launch(key(1, 1), Body::Bomb(bomb));
    assert!(field.detonate(1));
    let out = field.step(&world, &[]);
    assert!(out.contains(&(
        key(3, 1),
        Outcome::Destroyed {
            at: Vec3::new(250.0, 0.0, 0.0)
        }
    )));
    assert!(field.get(key(3, 2)).is_some());
    assert_eq!(field.len(), 1);
}

#[test]
fn hit_codes_round_trip() {
    for k in [
        HitKind::SpinOut,
        HitKind::Blasted,
        HitKind::Squished,
        HitKind::TntOnHead,
    ] {
        assert_eq!(HitKind::from_code(k.code()), Some(k));
    }
}
