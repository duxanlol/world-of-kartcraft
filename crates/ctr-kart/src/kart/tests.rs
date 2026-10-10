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
