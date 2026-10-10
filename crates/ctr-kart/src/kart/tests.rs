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
