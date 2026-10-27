use super::*;

/// A flat floor at y = 0, `half` lu each way, in 2000-lu tiles.
fn floor(half: f32) -> Vec<[[f32; 3]; 3]> {
    let mut tris = Vec::new();
    let tile = 2000.0;
    let n = (half * 2.0 / tile).ceil() as i32;
    for i in 0..n {
        for j in 0..n {
            let x0 = -half + i as f32 * tile;
            let z0 = -half + j as f32 * tile;
            let (x1, z1) = (x0 + tile, z0 + tile);
            tris.push([[x0, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z1]]);
            tris.push([[x0, 0.0, z0], [x1, 0.0, z1], [x1, 0.0, z0]]);
        }
    }
    tris
}

fn world(tris: Vec<[[f32; 3]; 3]>) -> Collision {
    Collision::new(tris, 160.0)
}

/// A wall across +Z at `z`, facing −Z.
fn wall(z: f32) -> Vec<[[f32; 3]; 3]> {
    let (w, h) = (4000.0, 400.0);
    vec![
        [[-w, 0.0, z], [w, h, z], [w, 0.0, z]],
        [[-w, 0.0, z], [-w, h, z], [w, h, z]],
    ]
}

const GAS: Input = Input {
    steer: 0.0,
    gas: true,
    brake: false,
    down: false,
    r1: false,
    l1: false,
};
const NONE: Modifiers = Modifiers {
    wumpa: 0,
    mask: false,
    super_engine: false,
};

fn run(kart: &mut Kart, w: &Collision, input: Input, frames: usize) {
    for _ in 0..frames {
        kart.tick(w, &input, &NONE);
    }
}

#[test]
fn full_gas_reaches_the_class_top_speed_in_about_forty_frames() {
    let w = world(floor(40000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
    let mut reached = None;
    for frame in 0..120 {
        kart.tick(&w, &GAS, &NONE);
        if reached.is_none() && kart.speed() >= 13140 - 200 {
            reached = Some(frame);
        }
    }
    let frame = reached.expect("never reached top speed");
    assert!((30..=55).contains(&frame), "top speed at frame {frame}");
    assert!((12900..=13300).contains(&kart.speed()), "{}", kart.speed());
    assert!(kart.grounded());
    // It drives along +Z (heading 0) on the floor, not into it.
    let p = kart.pose().position;
    assert!(p.z > 1000.0, "{p:?}");
    assert!(p.y.abs() < 5.0, "{p:?}");
}

#[test]
fn coasting_stops_and_braking_stops_sooner() {
    let w = world(floor(80000.0));
    let stop = |input: Input| {
        let mut kart = Kart::new(Class::Balanced, Vec3::ZERO, 0);
        run(&mut kart, &w, GAS, 90);
        (0..400)
            .position(|_| {
                kart.tick(&w, &input, &NONE);
                kart.speed() < 50
            })
            .expect("never stopped")
    };
    let coast = stop(Input::default());
    let brake = stop(Input {
        brake: true,
        ..Input::default()
    });
    assert!((80..=140).contains(&coast), "coast {coast}");
    assert!(brake < coast, "brake {brake} coast {coast}");
}

#[test]
fn steering_turns_left_and_bleeds_speed() {
    let w = world(floor(80000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::ZERO, 0);
    run(&mut kart, &w, GAS, 90);
    let top = kart.speed();
    run(&mut kart, &w, Input { steer: -1.0, ..GAS }, 30);
    // Left is + in CTR's angle; a second of full lock turns well over 45°.
    let turned = diff(kart.pose().heading, 0);
    assert!(turned > 512, "turned {turned}");
    assert!(kart.speed() < top - 1000, "{} vs {top}", kart.speed());
}

#[test]
fn a_hop_leaves_the_ground_and_lands_about_a_third_of_a_second_later() {
    let w = world(floor(40000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::ZERO, 0);
    run(&mut kart, &w, GAS, 10);
    kart.events.clear();
    kart.tick(&w, &Input { r1: true, ..GAS }, &NONE);
    assert!(kart.events.contains(&Event::Hop), "{:?}", kart.events);
    // CTR's probe counts the kart grounded within 16 lu of the floor, so the hop reads as
    // airborne only from its second frame; count from there to touchdown.
    let mut air = 0;
    let mut peak: f32 = 0.0;
    let mut left = false;
    for _ in 0..40 {
        kart.tick(&w, &GAS, &NONE);
        peak = peak.max(kart.pose().position.y);
        left |= !kart.grounded();
        if left && kart.grounded() {
            break;
        }
        air += 1;
    }
    assert!((7..=16).contains(&air), "air {air} frames");
    assert!((30.0..=60.0).contains(&peak), "apex {peak} lu");
    assert!(kart.events.iter().any(|e| matches!(e, Event::Land { .. })));
}

/// Hops into a left powerslide at speed and returns the kart mid-drift.
fn drifting(w: &Collision) -> Kart {
    let mut kart = Kart::new(Class::Balanced, Vec3::ZERO, 0);
    run(&mut kart, w, GAS, 90);
    let hold = Input {
        steer: -1.0,
        r1: true,
        ..GAS
    };
    kart.events.clear();
    for _ in 0..20 {
        kart.tick(w, &hold, &NONE);
        if kart.state == State::Drifting {
            break;
        }
    }
    assert_eq!(kart.state, State::Drifting, "{:?}", kart.events);
    assert!(kart.events.contains(&Event::DriftStart { left: true }));
    kart
}

#[test]
fn a_boost_in_the_red_zone_pays_and_one_too_early_does_not() {
    let w = world(floor(200000.0));
    let hold = Input {
        steer: -1.0,
        r1: true,
        ..GAS
    };
    let tap = Input { l1: true, ..hold };

    let mut kart = drifting(&w);
    // Wait for the red zone (the last 15 of 30 frames), then tap the other shoulder.
    loop {
        kart.tick(&w, &hold, &NONE);
        if kart.turbo_meter().is_some_and(|(_, red)| red) {
            break;
        }
    }
    run(&mut kart, &w, hold, 10);
    kart.events.clear();
    kart.tick(&w, &tap, &NONE);
    let boost = kart.events.iter().find_map(|e| match e {
        Event::Boost { stage, reserves } => Some((*stage, *reserves)),
        _ => None,
    });
    let (stage, reserves) = boost.expect("no boost");
    assert_eq!(stage, 1);
    assert!(reserves > 1000, "{reserves}");
    assert!(kart.reserves() > 0);
    // Turbo raises the target past the class's top speed.
    run(&mut kart, &w, hold, 20);
    assert!(kart.speed() > 13140, "{}", kart.speed());

    let mut early = drifting(&w);
    run(&mut early, &w, hold, 2);
    early.events.clear();
    early.tick(&w, &tap, &NONE);
    assert!(
        early.events.contains(&Event::BoostEarly),
        "{:?}",
        early.events
    );
    assert_eq!(early.reserves(), 0);
}

#[test]
fn letting_go_of_the_hop_ends_the_drift() {
    let w = world(floor(200000.0));
    let mut kart = drifting(&w);
    kart.tick(&w, &Input { steer: -1.0, ..GAS }, &NONE);
    assert_eq!(kart.state, State::Normal);
    assert!(kart.events.contains(&Event::DriftEnd));
}

#[test]
fn a_head_on_wall_slams_and_a_masked_kart_slides_along_it() {
    let mut tris = floor(40000.0);
    tris.extend(wall(3000.0));
    let w = world(tris);
    let mut kart = Kart::new(Class::Balanced, Vec3::ZERO, 0);
    let mut slammed = false;
    for _ in 0..200 {
        kart.tick(&w, &GAS, &NONE);
        slammed |= kart.events.iter().any(|e| matches!(e, Event::Slam { .. }));
        assert!(kart.pose().position.z < 3000.0, "through the wall");
    }
    assert!(slammed);

    let mut masked = Kart::new(Class::Balanced, Vec3::ZERO, 0);
    let mask = Modifiers { mask: true, ..NONE };
    for _ in 0..200 {
        masked.tick(&w, &GAS, &mask);
        assert!(!masked
            .events
            .iter()
            .any(|e| matches!(e, Event::Slam { .. })));
        assert!(masked.pose().position.z < 3000.0, "through the wall");
    }
}

#[test]
fn a_long_hop_off_a_ledge_pays_hang_time() {
    // A plateau at y = 800 ending at z = 4000 over a floor at y = 0.
    let mut tris = floor(60000.0);
    let (w, top) = (3000.0, 800.0);
    tris.push([[-w, top, -w], [-w, top, 4000.0], [w, top, 4000.0]]);
    tris.push([[-w, top, -w], [w, top, 4000.0], [w, top, -w]]);
    let world = world(tris);
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, top + 1.0, -2500.0), 0);
    let mut hopped = false;
    let mut paid = None;
    for _ in 0..300 {
        let at = kart.pose().position.z;
        let input = Input {
            r1: !hopped && at > 3900.0,
            ..GAS
        };
        hopped |= input.r1;
        kart.tick(&world, &input, &NONE);
        if let Some(Event::HangTime { level }) = kart
            .events
            .iter()
            .find(|e| matches!(e, Event::HangTime { .. }))
        {
            paid = Some(*level);
            break;
        }
    }
    assert!(hopped);
    assert!(paid.is_some(), "no hang time");
}

#[test]
fn the_stick_curve_matches_retail() {
    assert_eq!(stick_strength(0, 28), 0);
    assert_eq!(stick_strength(47, 28), 0);
    assert_eq!(stick_strength(127, 28), 28);
    // Halfway through the curve reads a fifth.
    let mid = stick_strength(48 + 47, 28);
    assert!((5..=6).contains(&mid), "{mid}");
}

#[test]
fn map_saturates_like_vehcalc() {
    assert_eq!(map(5, 0, 10, 0, 100), 50);
    assert_eq!(map(-1, 0, 10, 0, 100), 0);
    assert_eq!(map(11, 0, 10, 0, 100), 100);
    // A reversed old range saturates at once.
    assert_eq!(map(5, 0, -2560, 0, 300), 300);
}

#[test]
fn the_engine_idles_low_revs_with_the_throttle_and_whines_through_a_powerslide() {
    let w = world(floor(200000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::ZERO, 0);
    run(&mut kart, &w, Input::default(), 30);
    let (idle, _) = kart.engine_sound();
    assert!((60..=62).contains(&idle), "idle {idle}");
    run(&mut kart, &w, GAS, 90);
    // At the class's top speed, about ×1.15 (d ≈ 141), not the top of the range.
    let (cruise, _) = kart.engine_sound();
    assert!((135..=147).contains(&cruise), "cruise {cruise}");
    // The meter's rise lifts the pitch through a slide.
    let hold = Input {
        steer: -1.0,
        r1: true,
        ..GAS
    };
    for _ in 0..20 {
        kart.tick(&w, &hold, &NONE);
        if kart.state == State::Drifting {
            break;
        }
    }
    let early = kart.engine_sound().0;
    run(&mut kart, &w, hold, 25);
    assert!(
        kart.engine_sound().0 > early,
        "{} vs {early}",
        kart.engine_sound().0
    );
}

#[test]
fn karts_bump_apart_and_the_heavier_one_wins() {
    let w = world(floor(40000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::ZERO, 0);
    run(&mut kart, &w, GAS, 60);
    let ahead = kart.pose().position + Vec3::Z * 60.0;
    let other = OtherKart {
        position: ahead,
        velocity: Vec3::ZERO,
        weight: 300,
        ..Default::default()
    };
    let before = kart.speed();
    kart.events.clear();
    kart.tick_among(&w, &GAS, &NONE, &[other]);
    assert!(kart.events.iter().any(|e| matches!(e, Event::Bump { .. })));
    assert!(kart.speed() < before / 2, "{} vs {before}", kart.speed());
    // Far apart, nothing.
    let far = OtherKart {
        position: Vec3::new(5000.0, 0.0, 0.0),
        ..other
    };
    kart.events.clear();
    kart.tick_among(&w, &GAS, &NONE, &[far]);
    assert!(!kart.events.iter().any(|e| matches!(e, Event::Bump { .. })));
}

/// A kart 60 lu ahead of `kart`, standing, wearing what `armed` sets.
fn ahead_of(kart: &Kart, armed: impl FnOnce(&mut OtherKart)) -> OtherKart {
    let mut other = OtherKart {
        position: kart.pose().position + Vec3::Z * 60.0,
        weight: 256,
        id: 7,
        ..Default::default()
    };
    armed(&mut other);
    other
}

fn rammed(kart: &Kart) -> Option<Ram> {
    kart.events.iter().find_map(|e| match e {
        Event::Rammed { by: 7, ram } => Some(*ram),
        _ => None,
    })
}

fn rammer(kart: &Kart) -> Option<Ram> {
    kart.events.iter().find_map(|e| match e {
        Event::Rammer { other: 7, ram } => Some(*ram),
        _ => None,
    })
}

#[test]
fn a_masked_kart_ramming_us_blasts_us_and_a_plain_bump_does_not() {
    let w = world(floor(40000.0));
    let mut kart = cruising(&w);
    kart.tick_among(&w, &GAS, &NONE, &[ahead_of(&kart, |_| {})]);
    assert_eq!(rammed(&kart), None);
    assert_eq!(kart.reaction(), None);
    let mut kart = cruising(&w);
    kart.tick_among(&w, &GAS, &NONE, &[ahead_of(&kart, |o| o.masked = true)]);
    assert_eq!(rammed(&kart), Some(Ram::Mask));
    assert!(matches!(kart.reaction(), Some((HitKind::Blasted, _))));
    // Masked ourselves, nothing gets through; and ours blasts it.
    let mut kart = cruising(&w);
    kart.mask(false);
    kart.tick_among(&w, &GAS, &NONE, &[ahead_of(&kart, |o| o.masked = true)]);
    assert_eq!(rammed(&kart), None);
    assert_eq!(rammer(&kart), None, "both masked: neither attacks");
    kart.events.clear();
    kart.tick_among(&w, &GAS, &NONE, &[ahead_of(&kart, |_| {})]);
    assert_eq!(rammer(&kart), Some(Ram::Mask));
    assert!(kart.masked(), "the mask stays");
}

#[test]
fn a_shield_ram_blasts_the_other_and_is_spent() {
    let w = world(floor(40000.0));
    // Theirs into us: blasted.
    let mut kart = cruising(&w);
    kart.tick_among(&w, &GAS, &NONE, &[ahead_of(&kart, |o| o.shielded = true)]);
    assert_eq!(rammed(&kart), Some(Ram::Shield));
    assert!(matches!(kart.reaction(), Some((HitKind::Blasted, _))));
    // Ours into them: spent, popping, and we are not hurt.
    let mut kart = cruising(&w);
    kart.shield_up(true);
    kart.tick_among(&w, &GAS, &NONE, &[ahead_of(&kart, |_| {})]);
    assert_eq!(rammer(&kart), Some(Ram::Shield));
    assert!(kart.shield().is_none());
    assert!(saw(&kart, Event::ShieldPopped));
    assert_eq!(kart.reaction(), None);
    // Shield against shield: nothing.
    let mut kart = cruising(&w);
    kart.shield_up(false);
    kart.tick_among(&w, &GAS, &NONE, &[ahead_of(&kart, |o| o.shielded = true)]);
    assert_eq!((rammed(&kart), rammer(&kart)), (None, None));
    assert!(kart.shield().is_some());
}

#[test]
fn a_turbo_ram_over_the_threshold_squishes_and_high_jumps() {
    let w = world(floor(40000.0));
    // Their turbo into us at speed: squished. Standing still, the impact is under 0xA00.
    let mut kart = cruising(&w);
    let fast = |o: &mut OtherKart| {
        o.turbo = true;
        o.velocity = Vec3::new(0.0, 0.0, -2.0 * RAM_TURBO_IMPACT);
    };
    kart.tick_among(&w, &GAS, &NONE, &[ahead_of(&kart, fast)]);
    assert_eq!(rammed(&kart), Some(Ram::Turbo));
    assert!(matches!(kart.reaction(), Some((HitKind::Squished, _))));
    let mut still = Kart::new(Class::Balanced, Vec3::ZERO, 0);
    run(&mut still, &w, Input::default(), 10);
    let slow = |o: &mut OtherKart| {
        o.turbo = true;
        o.velocity = Vec3::new(0.0, 0.0, -0.5 * RAM_TURBO_IMPACT);
    };
    still.events.clear();
    still.tick_among(&w, &Input::default(), &NONE, &[ahead_of(&still, slow)]);
    assert_eq!(rammed(&still), None, "under the threshold");
    // Ours: under the turbo item at speed into a kart without one, we high-jump.
    let mut kart = cruising(&w);
    kart.fire_turbo(false);
    run(&mut kart, &w, GAS, 2);
    kart.events.clear();
    let y = kart.pose().position.y;
    kart.tick_among(&w, &GAS, &NONE, &[ahead_of(&kart, |_| {})]);
    assert_eq!(rammer(&kart), Some(Ram::Turbo));
    assert_eq!(kart.reaction(), None);
    run(&mut kart, &w, GAS, 8);
    let high = kart.pose().position.y - y;
    // A hop's 4596 rises about 140 lu in 8 frames; three times the force, far higher.
    let mut hop = cruising(&w);
    let y0 = hop.pose().position.y;
    hop.tick(&w, &Input { r1: true, ..GAS }, &NONE);
    run(&mut hop, &w, GAS, 8);
    let low = hop.pose().position.y - y0;
    assert!(high > 2.0 * low, "{high} vs a hop's {low}");
    // A turbo from a pad or a slide is no turbo item.
    let mut kart = cruising(&w);
    kart.start_boost(true);
    kart.tick_among(&w, &GAS, &NONE, &[ahead_of(&kart, |_| {})]);
    assert_eq!(rammer(&kart), None);
}

#[test]
fn a_cloud_slows_ices_or_takes_the_hop() {
    let w = world(floor(40000.0));
    let top = |cloud| {
        let mut kart = Kart::new(Class::Balanced, Vec3::ZERO, 0);
        kart.set_cloud(cloud);
        run(&mut kart, &w, GAS, 120);
        kart.speed()
    };
    let clear = top(None);
    let slowed = top(Some(CloudEffect::Slow));
    assert!(
        (clear - slowed - DAMAGED_SPEED).abs() < 300,
        "{clear} vs {slowed}"
    );
    let mut kart = cruising(&w);
    kart.set_cloud(Some(CloudEffect::Ice));
    run(&mut kart, &w, GAS, 1);
    assert_eq!(kart.terrain, Terrain::ICE);
    let mut kart = cruising(&w);
    kart.set_cloud(Some(CloudEffect::NoHop));
    kart.tick(&w, &Input { r1: true, ..GAS }, &NONE);
    assert!(!saw(&kart, Event::Hop));
    kart.set_cloud(None);
    run(&mut kart, &w, GAS, 1);
    kart.tick(&w, &Input { r1: true, ..GAS }, &NONE);
    assert!(saw(&kart, Event::Hop));
}

/// A kart at the class's top speed on a long floor.
fn cruising(w: &Collision) -> Kart {
    let mut kart = Kart::new(Class::Balanced, Vec3::ZERO, 0);
    run(&mut kart, w, GAS, 90);
    kart.events.clear();
    kart
}

fn saw(kart: &Kart, event: Event) -> bool {
    kart.events.contains(&event)
}

#[test]
fn a_spin_out_locks_out_thirty_frames_brakes_and_recentres() {
    let w = world(floor(40000.0));
    let mut kart = cruising(&w);
    kart.fire_turbo(false);
    assert!(kart.reserves() > 0);
    assert!(kart.hit(HitKind::SpinOut));
    // The boost is gone at once; the spin installs next frame.
    assert_eq!(kart.reserves(), 0);
    assert!(saw(
        &kart,
        Event::Hit {
            kind: HitKind::SpinOut
        }
    ));
    assert_eq!(kart.reaction(), Some((HitKind::SpinOut, 30)));
    let heading = kart.pose().heading;
    let start = kart.speed();
    let mut whirled = 0.0f32;
    for frame in 1..=30 {
        kart.tick(&w, &GAS, &NONE);
        assert!(matches!(kart.state, State::Spinning(_)), "frame {frame}");
        assert!(!kart.can_fire());
        whirled = whirled.max(kart.drawn_yaw_offset().abs());
    }
    assert!(saw(&kart, Event::SpinOut));
    assert_eq!(kart.reaction(), Some((HitKind::SpinOut, 0)));
    // Gas held, yet braked at BrakeFriction / 2 = 130 a frame.
    assert!(kart.speed() < start - 3000, "{} from {start}", kart.speed());
    assert!(whirled > 2.0, "the model whirls: {whirled}");
    // Then it winds back to face forward and drives on, on its old heading.
    let mut frames = 30;
    while matches!(kart.state, State::Spinning(_)) {
        kart.tick(&w, &GAS, &NONE);
        frames += 1;
        assert!(frames < 120, "never recentred");
    }
    assert_eq!(kart.drawn_yaw_offset(), 0.0);
    assert_eq!(kart.pose().heading, heading);
    assert!(kart.reaction().is_none());
    // A spin on a spinning kart only restarts its 30 frames.
    let mut again = cruising(&w);
    again.hit(HitKind::SpinOut);
    run(&mut again, &w, GAS, 10);
    assert!(again.hit(HitKind::SpinOut));
    assert_eq!(again.reaction(), Some((HitKind::SpinOut, 30)));
    assert!(again.pending.is_none());
}

#[test]
fn a_blast_bounces_for_seventy_five_frames_and_a_second_one_is_ignored() {
    let w = world(floor(40000.0));
    let mut kart = cruising(&w);
    kart.fire_turbo(true);
    assert!(kart.hit(HitKind::Blasted));
    assert_eq!(kart.reserves(), 0);
    // A second blast on the way in, and then mid-tumble, does nothing.
    assert!(!kart.hit(HitKind::Blasted));
    let mut peak = 0.0f32;
    let mut landings = 0;
    for frame in 1..=75 {
        kart.events.clear();
        kart.tick(&w, &GAS, &NONE);
        assert!(
            matches!(kart.reaction(), Some((HitKind::Blasted, _))),
            "frame {frame}"
        );
        assert!(!kart.can_fire());
        assert!(!kart.hit(HitKind::Blasted));
        peak = peak.max(kart.pose().position.y);
        landings += kart
            .events
            .iter()
            .filter(|e| matches!(e, Event::Land { .. }))
            .count();
        // Drawn tumbling round its pivot.
        assert!(kart.pose().offset.length() <= 2.0 * BLAST_PIVOT + 0.01);
    }
    assert_eq!(kart.reaction(), Some((HitKind::Blasted, 0)));
    // `6000 + 2·2400` = 10800 up at first, more than twice a hop: it flies high, more than once.
    assert!(peak > 100.0, "apex {peak} lu");
    assert!(landings >= 2, "{landings} landings");
    kart.tick(&w, &GAS, &NONE);
    assert!(kart.reaction().is_none());
    assert!(kart.can_fire());
    // The last bounce's landing squash settles back to the model as built.
    run(&mut kart, &w, GAS, 30);
    assert_eq!(kart.pose().scale, Vec3::ONE);
}

#[test]
fn a_squish_spins_and_flattens_and_slows_for_four_seconds() {
    let w = world(floor(40000.0));
    let mut kart = cruising(&w);
    assert!(kart.hit(HitKind::Squished));
    assert_eq!(kart.reaction(), Some((HitKind::Squished, 8)));
    kart.tick(&w, &GAS, &NONE);
    assert!(matches!(kart.state, State::Spinning(_)));
    assert_eq!(kart.squish_scale(), 0.0);
    assert_eq!(kart.pose().scale.y, 0.0);
    assert_eq!(kart.reaction(), Some((HitKind::Squished, 119)));
    // 240 ms of lockout, then it recentres and drives on, still flat and 4096 slower.
    run(&mut kart, &w, GAS, 100);
    assert_eq!(kart.state, State::Normal);
    assert!(matches!(kart.reaction(), Some((HitKind::Squished, _))));
    assert_eq!(kart.target_speed(&NONE), 13140 - 4096);
    run(&mut kart, &w, GAS, 18);
    assert!(kart.reaction().is_some());
    kart.tick(&w, &GAS, &NONE);
    assert!(kart.reaction().is_none());
    assert_eq!(kart.squish_scale(), 1.0);
}

/// Puts a TNT on a cruising kart seeded `seed` and drives until it sits on the head.
fn tnt_on_head(w: &Collision, seed: u16) -> Kart {
    let mut kart = cruising(w);
    kart.seed(seed);
    assert!(kart.hit(HitKind::TntOnHead));
    assert!(!kart.can_fire());
    for _ in 0..40 {
        if saw(&kart, Event::TntLanded) {
            break;
        }
        kart.tick(w, &GAS, &NONE);
    }
    assert!(saw(&kart, Event::TntLanded), "{:?}", kart.events);
    assert_eq!(kart.tnt_fuse(), Some(TNT_FUSE_FRAMES - 1));
    kart.events.clear();
    kart
}

/// Hops as fast as the kart allows; the hops it took to throw the TNT off, if it did.
fn shake(kart: &mut Kart, w: &Collision) -> Option<usize> {
    let mut hops = 0;
    for frame in 0..TNT_FUSE_FRAMES {
        let input = Input {
            r1: frame % 2 == 0,
            ..GAS
        };
        kart.tick(w, &input, &NONE);
        hops += kart.events.iter().filter(|e| **e == Event::Hop).count();
        if kart.events.contains(&Event::TntThrownOff) {
            return Some(hops);
        }
        assert!(!kart.events.contains(&Event::TntExploded));
        kart.events.clear();
    }
    None
}

#[test]
fn a_tnt_takes_eight_or_nine_hops_to_shake_off() {
    let w = world(floor(40000.0));
    // MixRNG from 0 gives 0x3619, odd: 7 more hops after the first.
    let mut kart = tnt_on_head(&w, 0);
    assert_eq!(kart.tnt.unwrap().jumps_left, 7);
    // Carrying it costs DamagedSpeed / 2.
    assert_eq!(kart.target_speed(&NONE), 13140 - 2048);
    assert_eq!(shake(&mut kart, &w), Some(8));
    assert!(kart.tnt_fuse().is_none());
    assert!(kart.can_fire());

    // From 1, 0x986E, even: 8 more.
    let mut kart = tnt_on_head(&w, 1);
    assert_eq!(kart.tnt.unwrap().jumps_left, 8);
    assert_eq!(shake(&mut kart, &w), Some(9));
}

#[test]
fn a_tnt_left_on_the_head_goes_off_after_ninety_frames() {
    let w = world(floor(40000.0));
    let mut kart = tnt_on_head(&w, 0);
    for frame in 2..=TNT_FUSE_FRAMES {
        kart.tick(&w, &GAS, &NONE);
        assert_eq!(
            kart.tnt_fuse(),
            Some(TNT_FUSE_FRAMES - frame),
            "frame {frame}"
        );
        assert!(!saw(&kart, Event::TntExploded));
    }
    kart.tick(&w, &GAS, &NONE);
    assert!(saw(&kart, Event::TntExploded));
    assert!(kart.tnt_fuse().is_none());
    // The caller's self-hit.
    assert!(kart.hit(HitKind::Blasted));

    // Blasted with it on: it goes off as the tumble starts.
    let mut kart = tnt_on_head(&w, 0);
    kart.hit(HitKind::Blasted);
    kart.tick(&w, &GAS, &NONE);
    assert!(saw(&kart, Event::TntExploded));
    // A second TNT blasts the kart and takes the first away.
    let mut kart = tnt_on_head(&w, 0);
    assert!(kart.hit(HitKind::TntOnHead));
    assert_eq!(kart.pending, Some(HitKind::Blasted));
    assert!(kart.tnt_fuse().is_none());
}

#[test]
fn a_shield_takes_one_hit_then_gives_twenty_one_invincible_frames() {
    let w = world(floor(40000.0));
    let mut kart = cruising(&w);
    kart.shield_up(false);
    assert_eq!(kart.shield().unwrap().frames_left, Some(360));
    assert!(!kart.hit(HitKind::Blasted));
    assert!(saw(&kart, Event::ShieldPopped));
    assert!(kart.shield().is_none());
    assert!(kart.reaction().is_none());
    for frame in 1..=20 {
        kart.tick(&w, &GAS, &NONE);
        assert!(!kart.hit(HitKind::SpinOut), "frame {frame}");
    }
    assert!(saw(&kart, Event::HitBlocked));
    kart.tick(&w, &GAS, &NONE);
    assert!(kart.hit(HitKind::SpinOut));

    // Green lasts 12 s; blue until used, and fired it is gone.
    let mut kart = cruising(&w);
    kart.shield_up(false);
    run(&mut kart, &w, GAS, 360);
    assert!(kart.shield().is_some_and(|s| s.alpha < 0.1));
    kart.tick(&w, &GAS, &NONE);
    assert!(kart.shield().is_none());
    assert!(saw(&kart, Event::ShieldExpired));
    kart.shield_up(true);
    run(&mut kart, &w, Input::default(), 1000);
    assert_eq!(kart.shield().map(|s| s.frames_left), Some(None));
    assert!(kart.fire_shield());
    assert!(kart.shield().is_none());
    assert!(!kart.fire_shield());
}

#[test]
fn a_mask_ignores_hits_speeds_the_kart_and_fades_after_eight_or_twelve_seconds() {
    let w = world(floor(50000.0));
    for (juiced, frames) in [(false, 240), (true, 360)] {
        let mut kart = cruising(&w);
        kart.mask(juiced);
        let plain = kart.target_speed(&NONE);
        run(&mut kart, &w, GAS, 1);
        assert!(kart.masked());
        assert!(!kart.hit(HitKind::Blasted));
        assert!(!kart.hit(HitKind::TntOnHead));
        assert!(saw(&kart, Event::HitBlocked));
        assert!(kart.reaction().is_none());
        let masked = Modifiers { mask: true, ..NONE };
        assert_eq!(kart.target_speed(&masked), plain + MASK_SPEED);
        run(&mut kart, &w, GAS, frames - 1);
        assert!(kart.masked());
        // Then 16 frames of fade, still protected.
        run(&mut kart, &w, GAS, 16);
        assert!(kart.masked());
        kart.tick(&w, &GAS, &NONE);
        assert!(!kart.masked(), "juiced {juiced}");
        assert!(saw(&kart, Event::MaskEnded));
        assert!(kart.hit(HitKind::SpinOut));
    }
}

#[test]
fn the_clock_spins_and_slows_by_race_position() {
    let w = world(floor(40000.0));
    assert_eq!(clock_slowdown(0), 5120);
    assert_eq!(clock_slowdown(7), 3328);
    let mut kart = cruising(&w);
    assert!(kart.clock(false, clock_slowdown(0)));
    assert_eq!(kart.clocked(), Some(240));
    run(&mut kart, &w, GAS, 100);
    assert_eq!(kart.state, State::Normal);
    assert!(!kart.can_fire());
    assert_eq!(kart.target_speed(&NONE), 13140 - 5120);
    run(&mut kart, &w, GAS, 140);
    assert!(kart.clocked().is_none());
    assert!(kart.can_fire());
    // A masked kart is not clocked.
    let mut kart = cruising(&w);
    kart.mask(false);
    assert!(!kart.clock(true, 5120));
    assert!(kart.clocked().is_none());
}

#[test]
fn the_turbo_item_is_seventy_five_frames_at_fire_level_128_or_256() {
    let w = world(floor(40000.0));
    let mut kart = cruising(&w);
    kart.fire_turbo(false);
    assert_eq!(kart.reserves(), 2400);
    assert_eq!(kart.fire_speed_cap, 3072);
    let mut kart = cruising(&w);
    kart.fire_turbo(true);
    assert_eq!(kart.reserves(), 2400);
    assert_eq!(kart.fire_speed_cap, 4096);
    run(&mut kart, &w, GAS, 74);
    assert!(kart.reserves() > 0);
    kart.tick(&w, &GAS, &NONE);
    assert_eq!(kart.reserves(), 0);
}

#[test]
fn held_on_the_grid_the_gas_revs_the_engine_and_the_kart_stands() {
    let w = world(floor(40000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
    kart.hold_on_grid(true);
    run(&mut kart, &w, Input::default(), 30);
    let (idle, _) = kart.engine_sound();
    // The gas revs it (`HOWL_Engine.c:170-181`): the pitch climbs well above idle...
    let mut pitch = idle;
    for _ in 0..8 {
        kart.tick(&w, &GAS, &NONE);
        assert!(kart.engine_sound().0 >= pitch, "the revs climb");
        pitch = kart.engine_sound().0;
    }
    assert!(pitch > idle + 40, "revved {pitch} vs idle {idle}");
    // ...the kart goes nowhere; the needle and the meter show the revs; the kart squashes...
    assert_eq!(kart.speed(), 0);
    let at = kart.pose().position;
    assert!(at.x.abs() < 1.0 && at.z.abs() < 1.0, "{at:?}");
    assert!(kart.needle() > kart.stats().accel_speed);
    assert!(kart.turbo_meter().is_some_and(|(_, red)| red));
    assert!(kart.pose().scale.y < 1.0);
    // ...and let go, it falls back.
    run(&mut kart, &w, Input::default(), 30);
    assert!(
        kart.engine_sound().0 < pitch - 40,
        "{:?}",
        kart.engine_sound()
    );
    assert_eq!(kart.needle(), 0);
    // Held too long, it over-revs (OtherFX 0xF) and GO gives nothing.
    let mut over = false;
    for _ in 0..20 {
        kart.tick(&w, &GAS, &NONE);
        over |= saw(&kart, Event::OverRev);
        kart.events.clear();
    }
    assert!(over);
    assert_eq!(kart.hold_on_grid(false), None);
    assert_eq!(kart.reserves(), 0);
    // Driving again: the gas moves it.
    run(&mut kart, &w, GAS, 10);
    assert!(kart.speed() > 0);
}

#[test]
fn revved_into_the_red_at_go_the_start_boost_fires() {
    let w = world(floor(40000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
    kart.hold_on_grid(true);
    run(&mut kart, &w, Input::default(), 10);
    run(&mut kart, &w, GAS, 6);
    kart.events.clear();
    assert_eq!(kart.hold_on_grid(false), Some(StartBoost::Small));
    assert!(saw(&kart, Event::Turbo));
    assert_eq!(kart.reserves(), crate::rev::START_BOOST_RESERVES);
    assert!(kart.revs().is_none());
    // Pumped twice first: the big one.
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
    kart.hold_on_grid(true);
    for _ in 0..2 {
        run(&mut kart, &w, GAS, 6);
        run(&mut kart, &w, Input::default(), 8);
    }
    run(&mut kart, &w, GAS, 8);
    assert_eq!(kart.hold_on_grid(false), Some(StartBoost::Big));
}

#[test]
fn a_drop_stretches_the_kart_falling_and_squashes_it_landing() {
    // Off a 400 lu ledge at speed onto the floor.
    let mut tris = floor(60000.0);
    let (w, top) = (3000.0, 400.0);
    tris.push([[-w, top, -w], [-w, top, 2000.0], [w, top, 2000.0]]);
    tris.push([[-w, top, -w], [w, top, 2000.0], [w, top, -w]]);
    let world = world(tris);
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, top + 1.0, -2500.0), 0);
    run(&mut kart, &world, GAS, 20);
    assert_eq!(kart.pose().scale, Vec3::ONE, "at rest on the plateau");
    let (mut tallest, mut flattest, mut landed) = (1.0f32, 1.0f32, None);
    let mut deepest = 0;
    for frame in 0..200 {
        kart.tick(&world, &GAS, &NONE);
        deepest = deepest.min(kart.squash);
        let s = kart.pose().scale;
        if !kart.grounded() && kart.pose().position.y < top - 1.0 {
            // Falling: taller and narrower (`VehPhysForce.c:1697-1708`).
            tallest = tallest.max(s.y);
            assert!(s.x <= 1.0 && s.z <= 1.0, "{s:?}");
        }
        if landed.is_none() && kart.grounded() && kart.pose().position.y < 1.0 {
            landed = Some(frame);
        }
        if landed.is_some_and(|l| frame - l < 10) {
            flattest = flattest.min(s.y);
        }
    }
    assert!(landed.is_some(), "never came down");
    // The fall's stretch up to 0x320 over 0xCCC. The landing's squash takes its least, −0x640,
    // settled 300 the same frame; the model eases 0xA0 a frame from tall to under its height.
    assert!(
        tallest > 1.1 && tallest <= 1.0 + 800.0 / 3276.0 + 1e-3,
        "{tallest}"
    );
    assert_eq!(deepest, SQUASH_LANDING_MIN + SQUASH_EASE);
    assert!(flattest < 0.95, "{flattest}");
    assert_eq!(kart.pose().scale, Vec3::ONE, "settled after");
}

#[test]
fn a_hop_crouches_first() {
    let w = world(floor(40000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::ZERO, 0);
    run(&mut kart, &w, GAS, 10);
    kart.tick(&w, &Input { r1: true, ..GAS }, &NONE);
    assert!(kart.events.contains(&Event::Hop));
    // `jumpSquishStretch` −800 on the hop's frame: the height eases down 0xA0 a frame.
    let s = kart.pose().scale;
    assert!((s.y - (3276.0 - 160.0) / 3276.0).abs() < 1e-4, "{s:?}");
    assert!(s.x > 1.0, "{s:?}");
}

#[test]
fn a_squish_springs_back_with_ctr_s_bounce() {
    let w = world(floor(40000.0));
    let mut kart = cruising(&w);
    assert!(kart.hit(HitKind::Squished));
    let mut restored = None;
    for frame in 0..200 {
        kart.events.clear();
        kart.tick(&w, &GAS, &NONE);
        if saw(&kart, Event::SquishRestored) {
            restored = Some(frame);
            break;
        }
        assert_eq!(kart.pose().scale.y, 0.0);
    }
    // `squishTimer` 0xF00 counts down 32 a frame: the 120th frame is the first unsquished.
    assert_eq!(restored, Some(119), "after the 120-frame squish");
    // Entries 1 to 14 of `bakedGteMath[SQUISH_RECOVER]`, the first lifted 3896/256 lu.
    let p = kart.pose();
    assert!((p.offset.y - 3896.0 / 256.0).abs() < 0.1, "{p:?}");
    let mut lifts = vec![p.offset.y];
    for _ in 1..14 {
        kart.tick(&w, &GAS, &NONE);
        lifts.push(kart.pose().offset.y);
    }
    let top = lifts.iter().cloned().fold(0.0f32, f32::max);
    assert!((top - 4636.0 / 256.0).abs() < 0.1, "{lifts:?}");
    assert_eq!(*lifts.last().unwrap(), 0.0);
    kart.tick(&w, &GAS, &NONE);
    assert!(kart.squish_recover.is_none());
    assert_eq!(kart.pose().offset, Vec3::ZERO);
    run(&mut kart, &w, GAS, 30);
    assert_eq!(kart.pose().scale, Vec3::ONE);
}

#[test]
fn only_more_than_0x2a0_ms_of_invincibility_flickers_and_every_other_frame() {
    let w = world(floor(40000.0));
    // A shield pop leaves exactly 0x2A0: no flicker (`231/RB_Player.c:242-245`).
    let mut kart = cruising(&w);
    kart.shield_up(false);
    assert!(!kart.hit(HitKind::Blasted));
    for _ in 0..25 {
        kart.tick(&w, &GAS, &NONE);
        assert!(!kart.flicker_hidden());
    }
    // Battle's 2880 after a tumble: hidden on odd frames until 0x2A0 is left.
    kart.invincible = 2880;
    let mut hidden = Vec::new();
    for _ in 0..90 {
        kart.tick(&w, &GAS, &NONE);
        hidden.push(kart.flicker_hidden());
    }
    let flickering = ((2880 - FLICKER_INVINCIBLE_MS) / MS) as usize;
    assert!(hidden[..flickering].windows(2).all(|p| p[0] != p[1]));
    assert!(hidden[flickering..].iter().all(|h| !h));
}

#[test]
fn a_powerslide_spin_out_is_its_own_event_and_a_hit_s_spin_is_not() {
    let w = world(floor(200000.0));
    let mut kart = drifting(&w);
    kart.drift_frames = DRIFT_FRAMES_TILL_SPINOUT + 1;
    kart.events.clear();
    kart.tick(
        &w,
        &Input {
            steer: -1.0,
            r1: true,
            ..GAS
        },
        &NONE,
    );
    assert!(matches!(kart.state, State::Spinning(_)));
    assert!(saw(&kart, Event::SpinOut));
    assert!(saw(&kart, Event::DriftSpinOut));

    let mut hit = cruising(&w);
    hit.hit(HitKind::SpinOut);
    hit.tick(&w, &GAS, &NONE);
    assert!(saw(&hit, Event::SpinOut));
    assert!(!saw(&hit, Event::DriftSpinOut));
}

#[test]
fn the_mask_grab_carries_the_kart_back_with_no_input_driving_it() {
    let w = world(floor(40000.0));
    let mut kart = cruising(&w);
    kart.shield_up(false);
    kart.events.clear();
    let fell = kart.pose().position;
    let ground = Vec3::new(100.0, 0.0, 100.0);
    kart.mask_grab(ground, 1024);
    assert!(saw(&kart, Event::MaskGrab));
    assert!(kart.grabbed());
    // Grabbed, the shield pops.
    kart.tick(&w, &GAS, &NONE);
    assert!(saw(&kart, Event::ShieldPopped) && kart.shield().is_none());
    assert_eq!((kart.speed(), kart.reserves()), (0, 0));
    // Everything held down: it neither moves nor turns, and nothing hits it.
    let wild = Input {
        steer: 1.0,
        gas: true,
        brake: false,
        down: false,
        r1: true,
        l1: false,
    };
    assert!(!kart.hit(HitKind::Blasted));
    let mut heights = vec![kart.pose().position.y];
    for _ in 1..44 {
        kart.tick(&w, &wild, &NONE);
        let p = kart.pose().position;
        assert_eq!((p.x, p.z), (fell.x, fell.z));
        assert_eq!(kart.pose().heading, 0);
        assert!(kart.revs().is_none());
        heights.push(p.y);
    }
    // Still for 25 frames while the mask comes down, then lifted 16 lu a frame.
    assert!(heights[..25].iter().all(|y| *y == fell.y));
    assert_eq!(heights[43], fell.y + 19.0 * 16.0);
    let mask = kart.grab_mask().unwrap();
    assert_eq!(mask.at, Some(kart.pose().position));
    // The 45th frame cuts to 128 lu over the respawn's ground, facing its heading, revving.
    kart.tick(&w, &wild, &NONE);
    let c = kart.camera_input();
    assert!(c.cut);
    assert_eq!(c.mask_grab, Some(64.0));
    assert!(!kart.camera_input().cut, "the cut is one shot");
    assert_eq!(kart.pose().position, ground + Vec3::Y * 126.0);
    assert_eq!(kart.pose().heading, 1024);
    assert!(kart.revs().is_some());
    assert_eq!(
        kart.grab_mask().unwrap().at,
        None,
        "the mask is back on the kart"
    );
    // The race's countdown cannot end Aku Aku's revs.
    assert_eq!(kart.hold_on_grid(false), None);
    // Lowered 2 lu a frame, the gas revving, for 32 more frames ...
    let mut revs = 0;
    for n in 1..=32 {
        kart.tick(&w, &wild, &NONE);
        assert_eq!(
            kart.pose().position,
            ground + Vec3::Y * (126.0 - 2.0 * n as f32)
        );
        assert!(kart.grabbed());
        revs = revs.max(kart.needle());
    }
    assert!(revs > 0, "the gas revs the engine");
    // ... then let go under 64 lu: that frame drives, and the kart falls to the ground.
    kart.events.clear();
    kart.tick(&w, &GAS, &NONE);
    assert!(!kart.grabbed() && kart.revs().is_none());
    assert_eq!(kart.camera_input().mask_grab, None);
    run(&mut kart, &w, GAS, 30);
    assert!(kart.grounded());
    assert!(kart.speed() > 0, "it drives off");
}

#[test]
fn the_camera_reads_the_blast_and_the_ground_under_the_kart() {
    let w = world(floor(40000.0));
    let mut kart = cruising(&w);
    let c = kart.camera_input();
    assert!(!c.blasted && c.grounded);
    assert!(c.ground.abs() < 1e-3);
    kart.hit(HitKind::Blasted);
    run(&mut kart, &w, GAS, 5);
    let c = kart.camera_input();
    assert!(c.blasted && !c.grounded, "{c:?}");
    assert!(c.ground.abs() < 1e-3, "the floor is in reach: {c:?}");
}

#[test]
fn a_powerslide_spin_out_keeps_the_reserves_and_a_hit_clears_them() {
    let w = world(floor(200000.0));
    let mut kart = drifting(&w);
    kart.reserves = 1500;
    kart.drift_frames = DRIFT_FRAMES_TILL_SPINOUT + 1;
    kart.tick(
        &w,
        &Input {
            steer: -1.0,
            r1: true,
            ..GAS
        },
        &NONE,
    );
    assert!(matches!(kart.state, State::Spinning(_)));
    // Drained a frame's worth, not wiped (`VehPhysProc.c:2859-2909` leaves them).
    assert_eq!(kart.reserves(), 1500 - MS);
    let mut hit = cruising(&w);
    hit.reserves = 1500;
    hit.hit(HitKind::SpinOut);
    hit.tick(&w, &GAS, &NONE);
    assert_eq!(hit.reserves(), 0);
}

#[test]
fn braking_on_a_turbo_pad_keeps_its_boost_and_off_it_clears_it() {
    let w = world(floor(40000.0));
    let brake = Input { brake: true, ..GAS };
    // On the pad, Square counts as let go while the boost lasts.
    let mut kart = cruising(&w);
    kart.turbo_pad();
    kart.tick(&w, &GAS, &NONE);
    let held = kart.reserves();
    assert!(held > 0);
    kart.turbo_pad();
    kart.tick(&w, &brake, &NONE);
    assert!(kart.reserves() > 0, "braking on the pad kept the boost");
    assert_eq!(kart.flags & BRAKE_WITH_ACCEL, 0);
    // Off it, the brake clears them.
    kart.tick(&w, &brake, &NONE);
    assert_eq!(kart.reserves(), 0);
}

#[test]
fn super_engine_tops_up_only_moving_on_the_gas_without_the_brake() {
    let w = world(floor(40000.0));
    let engine = Modifiers {
        super_engine: true,
        ..NONE
    };
    let mut kart = cruising(&w);
    kart.tick(&w, &GAS, &engine);
    assert!(kart.reserves() > 0);
    // A power brake (gas and brake) stops the top-up, and the brake clears what was there.
    kart.tick(&w, &Input { brake: true, ..GAS }, &engine);
    assert_eq!(kart.reserves(), 0);
}
#[test]
fn the_turbo_item_pops_the_nose_up_and_it_comes_down_after() {
    let w = world(floor(40000.0));
    let mut kart = cruising(&w);
    kart.fire_turbo(false);
    kart.tick(&w, &GAS, &NONE);
    assert_eq!(kart.wheelie(), Some((Wheelie::Start, 0)));
    // The nose is up in the drawn pose: the kart's forward axis points above level.
    run(&mut kart, &w, GAS, 12);
    assert_eq!(kart.wheelie(), Some((Wheelie::Hold, 0)));
    let forward = kart.pose().basis * Vec3::Z;
    assert!(forward.y > 0.3, "nose up: {forward:?}");
    // The boost over, it settles over the recovery's frames and is gone.
    while kart.reserves() > 0 {
        kart.tick(&w, &GAS, &NONE);
    }
    kart.tick(&w, &GAS, &NONE);
    assert!(matches!(kart.wheelie(), Some((Wheelie::Recover, _))));
    run(&mut kart, &w, GAS, 10);
    assert_eq!(kart.wheelie(), None);
}

#[test]
fn the_first_two_powerslide_boosts_give_no_wheelie() {
    let w = world(floor(200000.0));
    let mut kart = drifting(&w);
    kart.turbo_meter = 1;
    kart.tick(
        &w,
        &Input {
            steer: -1.0,
            r1: true,
            l1: true,
            ..GAS
        },
        &NONE,
    );
    assert!(kart.reserves() > 0, "a boost");
    assert_eq!(kart.wheelie(), None);
}

#[test]
fn a_cut_short_rise_comes_down_from_as_high_as_it_got() {
    // Halfway up the rise (5 of 0..10) is halfway down the recovery (4 of 0..8), and the top of
    // the rise is the recovery's first frame.
    assert_eq!(super::wheelie_remap(5, 11, 9), 4);
    assert_eq!(super::wheelie_remap(10, 11, 9), 0);
    assert_eq!(super::wheelie_remap(0, 11, 9), 8);
}

#[test]
fn every_wheelie_code_names_one_frame() {
    assert!(wheelie_frame(0).is_none() && wheelie_frame(22).is_none());
    for code in 1..=21u8 {
        let (turn, scale, _) = wheelie_frame(code).unwrap();
        assert!(
            turn.determinant() > 0.99 && scale.min_element() > 0.7,
            "{code}"
        );
    }
    // The hold is the nose highest up: its pitch is the table's largest.
    let pitch = |c| wheelie_frame(c).unwrap().0.z_axis.y;
    assert!((1..=21).all(|c| pitch(c) <= pitch(12) + 1e-6));
}

/// Reversing at full lock keeps half the reverse speed in every class (the turn's decrease is
/// capped there; CTR's would leave an Accel kart a tenth), and driving forward is untouched.
#[test]
fn a_full_lock_reverse_keeps_half_its_speed() {
    let w = world(floor(80000.0));
    let back = Input {
        steer: 1.0,
        gas: false,
        brake: false,
        down: true,
        r1: false,
        l1: false,
    };
    for class in [Class::Balanced, Class::Accel, Class::Speed, Class::Turn] {
        let mut kart = Kart::new(class, Vec3::new(0.0, 1.0, 0.0), 0);
        run(&mut kart, &w, back, 150);
        assert!(
            kart.speed() >= BACKWARD_SPEED / 2 - 64,
            "{class:?} reverses at {} turning",
            kart.speed()
        );
        // Forward at full lock still pays the whole decrease.
        let mut kart = Kart::new(class, Vec3::new(0.0, 1.0, 0.0), 0);
        run(&mut kart, &w, Input { steer: 1.0, ..GAS }, 150);
        let t = BACKWARD_TURN_RATE.min((kart.amp_turn >> 8).abs());
        let full = map(t, 0, BACKWARD_TURN_RATE, 0, class.stats().turn_decrease);
        assert!(
            kart.speed() <= class.stats().speed - full + 64,
            "{class:?} forward"
        );
    }
}

/// Stopped dead with the model still turned off its heading (a wall slam out of a turn), the
/// kart does not spin in place: the low-speed transfer (4.6) hands the offset to the heading
/// while the model recentres, so the drawn yaw settles where it was.
#[test]
fn a_stopped_kart_with_its_model_turned_settles_instead_of_spinning() {
    let w = world(floor(4000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
    run(&mut kart, &w, Input::default(), 10);
    kart.turn_angle = 400;
    let drawn = |k: &Kart| k.angle + k.turn_angle;
    let before = drawn(&kart);
    let mut travelled = 0;
    let mut prev = before;
    for _ in 0..90 {
        kart.tick(&w, &Input::default(), &NONE);
        travelled += diff(drawn(&kart), prev).abs();
        prev = drawn(&kart);
    }
    assert_eq!(kart.speed(), 0);
    assert_eq!(kart.turn_angle, 0, "the model recentres");
    assert!(
        travelled < 64,
        "the drawn kart turned {travelled} of 4096 standing still"
    );
}

/// A kart at full gas over `floor`, with water `surface` lu up (or none), after `frames`.
fn on_water(surface: Option<f32>, frames: usize) -> Kart {
    let w = world(floor(200000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
    kart.water = surface;
    run(&mut kart, &w, GAS, frames);
    kart
}

/// Deep water floats the kart at the surface less its draft, grounded, at the afloat row's
/// speed; shallow water leaves it on its bed; water under the ground changes nothing.
#[test]
fn deep_water_floats_the_kart_and_the_shallows_drive_their_bed() {
    let deep = on_water(Some(200.0), 150);
    assert!(deep.afloat() && deep.grounded(), "afloat");
    // At speed it rides up, skimming.
    assert!(
        (deep.pos.y - (200.0 - DRAFT_SKIMMING)).abs() < 2.0,
        "at {}",
        deep.pos.y
    );
    let top = Class::Balanced.stats().speed;
    let afloat = top * Terrain::AFLOAT.speed / 256;
    assert!(
        (deep.speed() - afloat).abs() < 400,
        "{} against {afloat}",
        deep.speed()
    );
    // The shallows: the floor (surface less draft) under the bed.
    let shallow = on_water(Some(6.0), 150);
    assert!(!shallow.afloat() && shallow.grounded());
    assert!(shallow.pos.y.abs() < 2.0, "on the bed: {}", shallow.pos.y);
    // Water under the ground is none: the same run as without any.
    let under = on_water(Some(-500.0), 150);
    let dry = on_water(None, 150);
    assert_eq!((under.pos, under.speed()), (dry.pos, dry.speed()));
}

/// Afloat at rest the kart sits its whole draft deep; at speed it rides up out of the water, as a
/// hovercraft does (DKR's ride height grows with speed).
#[test]
fn a_kart_rides_higher_on_water_the_faster_it_goes() {
    let w = world(floor(200000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
    kart.water = Some(200.0);
    run(&mut kart, &w, Input { gas: false, ..GAS }, 60);
    assert!(kart.afloat());
    assert!(
        (kart.pos.y - (200.0 - DRAFT)).abs() < 1.0,
        "at rest {}",
        kart.pos.y
    );
    run(&mut kart, &w, GAS, 150);
    assert!(kart.afloat());
    assert!(
        (kart.pos.y - (200.0 - DRAFT_SKIMMING)).abs() < 1.0,
        "skimming {}",
        kart.pos.y
    );
}

/// On the water a powerslide is CTR's: hop into it, and a tap in the red zone pays a turbo that
/// planes the kart, the three boosts as on the ground.
#[test]
fn a_powerslide_on_water_pays_ctr_s_turbos() {
    let w = world(floor(200000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
    kart.water = Some(200.0);
    run(&mut kart, &w, GAS, 90);
    assert!(kart.afloat());
    let hold = Input {
        steer: -1.0,
        r1: true,
        ..GAS
    };
    let tap = Input { l1: true, ..hold };
    for _ in 0..20 {
        kart.tick(&w, &hold, &NONE);
        if kart.state == State::Drifting {
            break;
        }
    }
    assert_eq!(kart.state, State::Drifting, "a powerslide on the water");
    let mut stages = Vec::new();
    for _ in 0..3 {
        while !kart.turbo_meter().is_some_and(|(_, red)| red) {
            kart.tick(&w, &hold, &NONE);
            assert_eq!(kart.state, State::Drifting);
        }
        run(&mut kart, &w, hold, 10);
        kart.events.clear();
        kart.tick(&w, &tap, &NONE);
        stages.extend(kart.events.iter().filter_map(|e| match e {
            Event::Boost { stage, .. } => Some(*stage),
            _ => None,
        }));
    }
    assert_eq!(stages, [1, 2, 3]);
    run(&mut kart, &w, hold, 2);
    assert!(kart.afloat() && kart.reserves() > 0, "planing on its turbo");
}

/// A drop onto the water dips the drawn kart under its floor and the water brings it back up
/// within a second; the floor itself never gives.
#[test]
fn a_drop_onto_water_dips_the_kart_and_it_bobs_back() {
    let w = world(floor(200000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1200.0, 0.0), 0);
    kart.water = Some(300.0);
    let idle = Input { gas: false, ..GAS };
    let mut deepest = 0.0f32;
    for _ in 0..90 {
        kart.tick(&w, &idle, &NONE);
        if kart.afloat() {
            deepest = deepest.min(kart.water_bob());
            assert!(
                (kart.pos.y - (300.0 - DRAFT)).abs() < 2.0,
                "the floor holds"
            );
        }
    }
    assert!(deepest < -6.0, "dipped {deepest}");
    run(&mut kart, &w, idle, 30);
    assert!(
        kart.water_bob().abs() <= BOB_LU + 0.5,
        "settled {}",
        kart.water_bob()
    );
}

/// Dropped from high over water, the kart lands on it, and a hop off it lands back on it.
#[test]
fn a_kart_lands_on_water_and_hops_off_it() {
    let w = world(floor(200000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1200.0, 0.0), 0);
    kart.water = Some(300.0);
    kart.events.clear();
    run(&mut kart, &w, GAS, 120);
    assert!(kart.afloat(), "on the water at {}", kart.pos.y);
    assert!(kart.events.iter().any(|e| matches!(e, Event::Land { .. })));
    assert!(
        (kart.pos.y - (300.0 - kart.draft())).abs() < 2.0,
        "{}",
        kart.pos.y
    );
    kart.events.clear();
    kart.tick(&w, &Input { r1: true, ..GAS }, &NONE);
    run(&mut kart, &w, GAS, 3);
    assert!(!kart.grounded(), "the hop leaves the water");
    run(&mut kart, &w, GAS, 40);
    assert!(kart.afloat() && kart.events.iter().any(|e| matches!(e, Event::Land { .. })));
}

/// A boost on the water planes the kart: faster than afloat alone.
#[test]
fn a_boost_planes_the_kart_on_water() {
    let w = world(floor(200000.0));
    let mut plain = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
    plain.water = Some(200.0);
    run(&mut plain, &w, GAS, 150);
    let mut boosted = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
    boosted.water = Some(200.0);
    run(&mut boosted, &w, GAS, 150);
    boosted.fire_turbo(false);
    run(&mut boosted, &w, GAS, 20);
    run(&mut plain, &w, GAS, 20);
    assert!(boosted.reserves() > 0 || boosted.speed() > plain.speed());
    assert!(
        boosted.speed() > plain.speed(),
        "{} > {}",
        boosted.speed(),
        plain.speed()
    );
}

/// How far (degrees) the kart turns in two seconds from rest on the flat, full left, the gas on
/// for `on` frames and off for `off` (`off` 0: held).
fn turned_mashing(on: usize, off: usize) -> f32 {
    let w = world(floor(40000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
    let mut total = 0;
    let mut last = kart.angle;
    for frame in 0..60 {
        let gas = off == 0 || frame % (on + off) < on;
        kart.tick(
            &w,
            &Input {
                steer: -1.0,
                gas,
                ..GAS
            },
            &NONE,
        );
        let d = (kart.angle - last).rem_euclid(4096);
        total += if d > 2048 { d - 4096 } else { d };
        last = kart.angle;
    }
    total.abs() as f32 * 360.0 / 4096.0
}

#[test]
fn mashing_the_gas_with_the_steer_held_spins_the_kart_round() {
    // CTR's accel tap (`VehPhysProc.c:1328-1357`, `:1424-1436`): held, the kart turns a third
    // of a circle in two seconds; mashed, it pivots round most of the way and more.
    let held = turned_mashing(1, 0);
    let mashed = turned_mashing(2, 2);
    assert!((90.0..130.0).contains(&held), "held: {held:.0}°");
    assert!(mashed > 2.5 * held, "mashed: {mashed:.0}°, held {held:.0}°");
}

/// A plateau `height` lu up for z < 0, a cliff at z = 0, and the flat a long way out below it.
fn cliff(height: f32) -> Collision {
    let mut tris = Vec::new();
    let tile = 2000.0;
    for i in -3..3 {
        for j in -6..0 {
            let (x0, z0) = (i as f32 * tile, j as f32 * tile);
            let (x1, z1) = (x0 + tile, z0 + tile);
            tris.push([[x0, height, z0], [x0, height, z1], [x1, height, z1]]);
            tris.push([[x0, height, z0], [x1, height, z1], [x1, height, z0]]);
        }
    }
    for i in -20..20 {
        for j in 0..120 {
            let (x0, z0) = (i as f32 * tile, j as f32 * tile);
            let (x1, z1) = (x0 + tile, z0 + tile);
            tris.push([[x0, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z1]]);
            tris.push([[x0, 0.0, z0], [x1, 0.0, z1], [x1, 0.0, z0]]);
        }
    }
    world(tris)
}

/// Full gas off the plateau's edge from far enough back to be at top speed; returns the kart
/// the frame it went over.
fn off_the_cliff(w: &Collision, height: f32) -> Kart {
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, height + 1.0, -9000.0), 0);
    for _ in 0..600 {
        kart.tick(w, &GAS, &NONE);
        if !kart.grounded() && kart.pos.z > 0.0 {
            return kart;
        }
    }
    panic!("never left the plateau: {:?}", kart.pos);
}

/// Off a cliff the wings open and the kart glides: far longer in the air than a fall, landing
/// well out, and it drives on from the touchdown at speed.
#[test]
fn off_a_cliff_the_kart_takes_off_and_glides_down() {
    let w = cliff(9000.0);
    let mut kart = off_the_cliff(&w, 9000.0);
    kart.events.clear();
    let mut frames = 0;
    let mut took_off = false;
    while frames < 900 {
        kart.tick(&w, &GAS, &NONE);
        frames += 1;
        took_off |= kart.events.contains(&Event::TakeOff);
        if took_off && kart.grounded() {
            break;
        }
    }
    assert!(took_off, "the wings opened");
    assert!(
        kart.grounded() && !kart.flying(),
        "down after {frames} frames"
    );
    assert!(kart.events.contains(&Event::Touchdown));
    // A fall of 9000 lu takes about 70 frames; the glide several times that.
    assert!(frames > 200, "glided {frames} frames");
    assert!(kart.pos.z > 25000.0, "out to {}", kart.pos.z);
    run(&mut kart, &w, GAS, 2);
    assert!(
        kart.speed() > Class::Balanced.stats().speed / 2,
        "drives on at {}",
        kart.speed()
    );
}

/// A turbo pushes a flying kart along the nose: turbos fired through a flight keep it higher
/// than a glide alone.
#[test]
fn a_turbo_keeps_a_flying_kart_up() {
    let w = cliff(9000.0);
    let fly = |turbo: bool| {
        let mut kart = off_the_cliff(&w, 9000.0);
        while !kart.flying() {
            kart.tick(&w, &GAS, &NONE);
        }
        // Out of the dive and into the glide first.
        run(&mut kart, &w, GAS, 60);
        let from = kart.pos.y;
        for frame in 0..90 {
            if turbo && frame % 20 == 0 {
                kart.fire_turbo(false);
            }
            kart.tick(&w, &GAS, &NONE);
        }
        assert!(kart.flying(), "still up");
        kart.pos.y - from
    };
    let (glide, powered) = (fly(false), fly(true));
    assert!(powered > glide + 300.0, "{powered} against {glide}");
}

/// No powerslide in flight: the drift's button banks harder, but no meter fills and no turbo is
/// paid, so a flight burns only what it took off with; a powerslide carried off the edge ends.
#[test]
fn a_flight_has_no_powerslide() {
    let w = cliff(12000.0);
    let mut kart = off_the_cliff(&w, 12000.0);
    while !kart.flying() {
        kart.tick(&w, &GAS, &NONE);
    }
    let steer = Input { steer: -1.0, ..GAS };
    run(&mut kart, &w, steer, 2);
    let hold = Input { r1: true, ..steer };
    let tap = Input { l1: true, ..hold };
    kart.events.clear();
    for frame in 0..90 {
        let input = if frame % 10 == 9 { tap } else { hold };
        kart.tick(&w, &input, &NONE);
        assert_ne!(kart.state, State::Drifting, "no powerslide in the air");
        assert!(kart.turbo_meter().is_none());
    }
    assert!(!kart.events.iter().any(|e| matches!(e, Event::Boost { .. })));
    assert_eq!(kart.reserves(), 0);
    let (_, bank) = kart.flight_attitude().unwrap();
    assert!(bank > 0.6, "banked hard left: {bank}");

    // Mid-powerslide off the plateau's edge: the slide ends as the wings open.
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 12001.0, -9000.0), 0);
    run(&mut kart, &w, GAS, 90);
    let slide = Input {
        steer: -0.3,
        r1: true,
        ..GAS
    };
    for _ in 0..400 {
        kart.tick(&w, &slide, &NONE);
        if kart.flying() {
            break;
        }
    }
    assert!(kart.flying(), "off the edge and flying");
    assert_ne!(kart.state, State::Drifting);
}

/// Pulled up the nose climbs, spending speed, until too slow for the wing: it stalls and sinks.
#[test]
fn pulled_up_the_kart_climbs_then_stalls() {
    let w = cliff(9000.0);
    let mut kart = off_the_cliff(&w, 9000.0);
    while !kart.flying() {
        kart.tick(&w, &GAS, &NONE);
    }
    // Out of the dive into the glide, then a turbo and the stick back.
    run(&mut kart, &w, GAS, 80);
    kart.fire_turbo(false);
    let up = Input { down: true, ..GAS };
    let start = kart.pos.y;
    let mut top = start;
    for _ in 0..200 {
        kart.tick(&w, &up, &NONE);
        top = top.max(kart.pos.y);
    }
    assert!(top > start + 100.0, "climbed from {start} to {top}");
    assert!(
        kart.pos.y < top - 100.0,
        "stalled, sinking from {top} to {}",
        kart.pos.y
    );
}

/// A flight over water lands on it: the kart hovers from the touchdown.
#[test]
fn a_flight_lands_on_water() {
    let w = cliff(9000.0);
    let mut kart = off_the_cliff(&w, 9000.0);
    kart.water = Some(1500.0);
    for _ in 0..900 {
        kart.tick(&w, &GAS, &NONE);
        if kart.events.contains(&Event::Touchdown) {
            break;
        }
    }
    assert!(
        kart.afloat() && !kart.flying(),
        "on the water at {}",
        kart.pos.y
    );
}

/// Off a ledge a CTR jump stays one (its hang time is CTR's); a double tap of the hop button in
/// the air opens the wings, a single tap does not.
#[test]
fn a_double_tap_in_the_air_opens_the_wings() {
    let w = cliff(2000.0);
    let mut kart = off_the_cliff(&w, 2000.0);
    let tap = Input { r1: true, ..GAS };
    run(&mut kart, &w, GAS, 6);
    kart.tick(&w, &tap, &NONE);
    run(&mut kart, &w, GAS, 3);
    assert!(!kart.flying(), "one tap buffers a hop");
    kart.tick(&w, &tap, &NONE);
    run(&mut kart, &w, GAS, 2);
    kart.tick(&w, &tap, &NONE);
    assert!(kart.flying(), "a double tap opens the wings");
    // And a plain drive off the same ledge lands as CTR's would.
    let mut plain = off_the_cliff(&w, 2000.0);
    for _ in 0..120 {
        plain.tick(&w, &GAS, &NONE);
        assert!(!plain.flying());
        if plain.grounded() {
            break;
        }
    }
    assert!(plain.grounded());
}

/// On the flat a hop's double tap opens the wings at its top; on a turbo the kart climbs away,
/// and without one it glides back down to the ground.
#[test]
fn a_kart_takes_off_from_a_hop_on_the_flat() {
    let w = world(floor(200000.0));
    let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
    run(&mut kart, &w, GAS, 90);
    kart.fire_turbo(false);
    let tap = Input { r1: true, ..GAS };
    kart.tick(&w, &tap, &NONE);
    let mut opened = false;
    for frame in 0..20 {
        let input = if frame % 4 < 2 { tap } else { GAS };
        kart.tick(&w, &input, &NONE);
        if kart.flying() {
            opened = true;
            break;
        }
    }
    assert!(opened, "the wings opened off the hop");
    let from = kart.pos.y;
    run(&mut kart, &w, GAS, 30);
    assert!(
        kart.pos.y > from,
        "climbing on its turbo: {from} to {}",
        kart.pos.y
    );
    for _ in 0..600 {
        kart.tick(&w, &GAS, &NONE);
        if kart.grounded() {
            break;
        }
    }
    assert!(
        kart.grounded() && !kart.flying(),
        "back down at {}",
        kart.pos.y
    );
}

/// On the water a hovercraft carries its momentum: the same turn at speed leaves its travel
/// further behind its nose than on the road.
#[test]
fn a_turn_on_water_slides_wider_than_on_the_road() {
    let w = world(floor(200000.0));
    let slip = |water: Option<f32>| {
        let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
        kart.water = water;
        run(&mut kart, &w, GAS, 120);
        let turn = Input { steer: -1.0, ..GAS };
        let mut most = 0;
        for _ in 0..30 {
            kart.tick(&w, &turn, &NONE);
            most = most.max(wrap_signed(kart.vel_yaw - kart.angle).abs());
        }
        most
    };
    let (road, water) = (slip(None), slip(Some(200.0)));
    assert!(
        water > road * 2 + 20,
        "water slips {water}, the road {road}"
    );
}

/// A hop off the water goes higher than one off the road (DKR's hovercraft).
#[test]
fn a_hop_off_the_water_goes_higher() {
    let w = world(floor(200000.0));
    let apex = |water: Option<f32>| {
        let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
        kart.water = water;
        run(&mut kart, &w, GAS, 60);
        let from = kart.pos.y;
        kart.tick(&w, &Input { r1: true, ..GAS }, &NONE);
        let mut top = from;
        for _ in 0..20 {
            kart.tick(&w, &GAS, &NONE);
            top = top.max(kart.pos.y);
        }
        top - from
    };
    let (road, water) = (apex(None), apex(Some(200.0)));
    assert!(water > road * 1.2, "off the water {water}, the road {road}");
}

/// A powerslide on the water leaves no skid marks: the water takes none.
#[test]
fn the_water_takes_no_skid_marks() {
    let w = world(floor(200000.0));
    let slide = |water: Option<f32>| {
        let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0);
        kart.water = water;
        run(&mut kart, &w, GAS, 90);
        let hold = Input {
            steer: -1.0,
            r1: true,
            ..GAS
        };
        let mut marked = false;
        for _ in 0..40 {
            kart.tick(&w, &hold, &NONE);
            marked |= kart.skids() != (false, false);
        }
        assert_eq!(kart.state, State::Drifting);
        marked
    };
    assert!(slide(None), "the road takes them");
    assert!(!slide(Some(200.0)), "the water does not");
}

/// With flight not allowed (a race) the wings never open, a double tap or a long fall; taken away
/// mid-flight, the wings fold and the kart comes down as CTR's.
#[test]
fn a_kart_without_flight_never_takes_off() {
    let w = cliff(9000.0);
    let mut kart = off_the_cliff(&w, 9000.0);
    kart.flight_allowed = false;
    let tap = Input { r1: true, ..GAS };
    for frame in 0..200 {
        let input = if frame % 4 < 2 { tap } else { GAS };
        kart.tick(&w, &input, &NONE);
        assert!(!kart.flying(), "frame {frame}");
        if kart.grounded() {
            break;
        }
    }
    assert!(kart.grounded(), "down as CTR's");

    let mut kart = off_the_cliff(&w, 9000.0);
    while !kart.flying() {
        kart.tick(&w, &GAS, &NONE);
    }
    kart.events.clear();
    kart.flight_allowed = false;
    kart.tick(&w, &GAS, &NONE);
    assert!(!kart.flying() && kart.events.contains(&Event::Touchdown));
}
