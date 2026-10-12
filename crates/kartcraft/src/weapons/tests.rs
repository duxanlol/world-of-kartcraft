use super::wire::{CrateSpot, Shot};
use super::*;

/// Where the session's lu are measured from, Bevy world.
const ORIGIN: Vec3 = Vec3::new(-120.0, 60.0, 8900.0);
const ME: KartId = 7;
const THEM: KartId = 9;

fn flat() -> Collision {
    let r = 40_000.0;
    Collision::new(
        [
            [[-r, 0.0, -r], [-r, 0.0, r], [r, 0.0, r]],
            [[-r, 0.0, -r], [r, 0.0, r], [r, 0.0, -r]],
        ],
        // Coarse enough that the grid keeps two faces this big.
        4096.0,
    )
}

/// Our kart settled on the plane at the origin, facing +Z, and the weapons with another racer
/// 60 yards off.
fn setup() -> (Weapons, Collision, Kart) {
    let world = flat();
    let mut kart = Kart::new(ctr_kart::Class::Balanced, Vec3::Y * 4.0, 0);
    for _ in 0..10 {
        kart.tick(
            &world,
            &ctr_kart::Input::default(),
            &ctr_kart::Modifiers::default(),
        );
    }
    let mut w = Weapons {
        me: ME,
        ..Default::default()
    };
    w.other(THEM, ORIGIN + Vec3::X * 60.0, 0.0, 0, "cortex", false);
    (w, world, kart)
}

fn tick(w: &mut Weapons, world: &Collision, kart: &mut Kart, n: usize) {
    for _ in 0..n {
        kart.events.clear();
        kart.tick(
            world,
            &ctr_kart::Input::default(),
            &ctr_kart::Modifiers::default(),
        );
        w.tick(world, kart, ORIGIN);
    }
}

fn shot(shooter: KartId, item: Item, shot: u16, at: Vec3) -> Down {
    Down::Shot(Shot {
        shooter,
        item,
        juiced: false,
        shot,
        aim: Aim {
            at: bevy_to_wow(at),
            yaw: yaw_of(0),
            ..Default::default()
        },
    })
}

#[test]
fn a_crate_is_claimed_once_rolled_and_fired_from_the_relay() {
    let (mut w, world, mut kart) = setup();
    let here = from_lu(kart.pose().position, ORIGIN);
    w.receive(Down::Layout {
        first: 0,
        crates: vec![CrateSpot {
            kind: CrateKind::Weapon,
            at: bevy_to_wow(here),
        }],
    });
    tick(&mut w, &world, &mut kart, 1);
    assert_eq!(std::mem::take(&mut w.outbox), vec![(ME, Up::Claim(0))]);
    tick(&mut w, &world, &mut kart, 5);
    assert!(w.outbox.is_empty(), "asked once while the server answers");

    // The server: taken by us, then the roll.
    w.receive(Down::Taken { index: 0, by: ME });
    tick(&mut w, &world, &mut kart, 1);
    assert!(!w.crates()[0].up);
    assert_eq!(w.held(), HeldView::Spinning);
    assert_eq!(w.hud_item(), Some((0x10, 0)));
    w.receive(Down::Item {
        item: Item::Missile,
        shots: 1,
    });
    tick(&mut w, &world, &mut kart, usize::from(ROLL_FRAMES) + 1);
    assert_eq!(w.held(), HeldView::Item(Item::Missile));
    assert_eq!(w.hud_item(), Some((Item::Missile.id(), 0)));

    // ○: a `W S` from where we are, nothing launched until the relay.
    w.press(true, false);
    tick(&mut w, &world, &mut kart, 1);
    let here = from_lu(kart.pose().position, ORIGIN);
    let out = std::mem::take(&mut w.outbox);
    let [(ME, Up::Fire { shot: 1, aim })] = out[..] else {
        panic!("{out:?}");
    };
    assert!(!aim.backward && !aim.dropped);
    assert!((wow_to_bevy(aim.at) - here).length() < 0.05);
    assert_eq!(w.shots().count(), 0);
    w.receive(shot(ME, Item::Missile, 1, here));
    tick(&mut w, &world, &mut kart, 1);
    assert_eq!(w.shots().count(), 1);

    // The crate comes back and regrows; full again under our kart, it breaks again.
    w.receive(Down::Back { index: 0 });
    tick(&mut w, &world, &mut kart, 1);
    assert!(w.crates()[0].up && w.crate_scale(&w.crates()[0]) < 1.0);
    tick(&mut w, &world, &mut kart, CRATE_GROW as usize - 1);
    assert!(
        w.crates()[0].up && w.outbox.is_empty(),
        "not hit while it grows"
    );
    tick(&mut w, &world, &mut kart, 1);
    assert!(!w.crates()[0].up);
    assert_eq!(std::mem::take(&mut w.outbox), vec![(ME, Up::Claim(0))]);
}

/// A crate's draw size, 0 broken to 1 full.
fn scale_of(w: &Weapons, i: usize) -> f32 {
    w.crate_scale(&w.crates()[i])
}

#[test]
fn a_crate_breaks_the_frame_we_touch_it() {
    let (mut w, world, mut kart) = setup();
    let here = from_lu(kart.pose().position, ORIGIN);
    w.receive(Down::Layout {
        first: 0,
        crates: vec![CrateSpot {
            kind: CrateKind::Fruit,
            at: bevy_to_wow(here),
        }],
    });
    tick(&mut w, &world, &mut kart, 1);
    // Gone at once, with its debris, before the server answers.
    assert_eq!(std::mem::take(&mut w.outbox), vec![(ME, Up::Claim(0))]);
    assert_eq!(scale_of(&w, 0), 0.0);
    assert_eq!(w.effects().len(), 1);
    assert_eq!(w.effects()[0].model, "crate_explosion");
    // The server's `X` keeps it broken, and breaks it no second time.
    w.receive(Down::Taken { index: 0, by: ME });
    tick(&mut w, &world, &mut kart, CLAIM_WAIT as usize + 5);
    assert_eq!(scale_of(&w, 0), 0.0);
    assert_eq!(w.effects().len(), 0, "one debris, played out");
    assert!(w.outbox.is_empty());
}

#[test]
fn a_refused_claim_brings_the_crate_back() {
    let (mut w, world, mut kart) = setup();
    let here = from_lu(kart.pose().position, ORIGIN);
    w.receive(Down::Layout {
        first: 0,
        crates: vec![CrateSpot {
            kind: CrateKind::Weapon,
            at: bevy_to_wow(here),
        }],
    });
    tick(&mut w, &world, &mut kart, 1);
    assert_eq!(std::mem::take(&mut w.outbox), vec![(ME, Up::Claim(0))]);
    // No answer: broken until the wait is out, then back and regrowing.
    tick(&mut w, &world, &mut kart, CLAIM_WAIT as usize - 1);
    assert!(!w.crates()[0].up && w.outbox.is_empty());
    tick(&mut w, &world, &mut kart, 1);
    assert!(w.crates()[0].up);
    assert!(scale_of(&w, 0) < 1.0);
    // Full again, the kart still on it: broken and asked for again.
    tick(&mut w, &world, &mut kart, CRATE_GROW as usize);
    assert_eq!(std::mem::take(&mut w.outbox), vec![(ME, Up::Claim(0))]);
}

#[test]
fn a_crate_another_takes_breaks_and_regrows_over_16_frames() {
    let (mut w, world, mut kart) = setup();
    let far = from_lu(kart.pose().position, ORIGIN) + Vec3::X * 30.0;
    w.receive(Down::Layout {
        first: 0,
        crates: vec![CrateSpot {
            kind: CrateKind::Weapon,
            at: bevy_to_wow(far),
        }],
    });
    tick(&mut w, &world, &mut kart, 1);
    assert_eq!(scale_of(&w, 0), 1.0);
    w.receive(Down::Taken { index: 0, by: THEM });
    tick(&mut w, &world, &mut kart, 1);
    assert_eq!(scale_of(&w, 0), 0.0);
    assert_eq!(w.effects().len(), 1);
    assert_eq!(w.held(), HeldView::Empty, "their crate gives us nothing");
    // The server's `U` after the 30-frame cooldown: +1/16 a frame to full (`RB_Crate.c:48-75`).
    tick(&mut w, &world, &mut kart, 29);
    assert_eq!(scale_of(&w, 0), 0.0);
    w.receive(Down::Back { index: 0 });
    tick(&mut w, &world, &mut kart, 1);
    assert_eq!(scale_of(&w, 0), 0.0);
    tick(&mut w, &world, &mut kart, 8);
    assert_eq!(scale_of(&w, 0), 0.5);
    tick(&mut w, &world, &mut kart, 8);
    assert_eq!(scale_of(&w, 0), 1.0);
    assert!(w.outbox.is_empty());
}

#[test]
fn nitro_and_tnt_stand_as_big_as_the_crates_and_fruit_is_three_quarters() {
    use super::look::{crate_model, mine_model, CRATE_SIZE};
    let (_, question) = crate_model(CrateKind::Weapon);
    assert_eq!(question, CRATE_SIZE);
    assert_eq!(crate_model(CrateKind::Fruit), ("crate_fruit", CRATE_SIZE));
    assert_eq!(mine_model(MineKind::Tnt), ("tnt", question));
    assert_eq!(mine_model(MineKind::Nitro), ("nitro", question));
    assert_eq!(crate_model(CrateKind::Wumpa), ("fruit", 0.75));
}

#[test]
fn a_crate_while_holding_gives_no_roulette() {
    let (mut w, world, mut kart) = setup();
    w.receive(Down::Item {
        item: Item::Bomb3,
        shots: 3,
    });
    tick(&mut w, &world, &mut kart, usize::from(ROLL_FRAMES) + 1);
    assert_eq!(w.held(), HeldView::Item(Item::Bomb3));
    assert_eq!(w.hud_item(), Some((Item::Bomb3.id(), 3)));
    w.receive(Down::Layout {
        first: 0,
        crates: vec![CrateSpot {
            kind: CrateKind::Weapon,
            at: [0.0; 3],
        }],
    });
    w.receive(Down::Taken { index: 0, by: ME });
    tick(&mut w, &world, &mut kart, 1);
    assert_eq!(w.held(), HeldView::Item(Item::Bomb3));
}

#[test]
fn a_mine_we_drive_into_is_ours_to_report() {
    let (mut w, world, mut kart) = setup();
    let here = from_lu(kart.pose().position, ORIGIN);
    // Their TNT dropped where we stand: it jumps onto our head, and we say so.
    w.receive(shot(THEM, Item::Tnt, 4, here));
    tick(&mut w, &world, &mut kart, 2);
    assert_eq!(
        std::mem::take(&mut w.outbox),
        vec![(
            ME,
            Up::Hit {
                shooter: THEM,
                shot: 4,
                kind: HitKind::TntOnHead
            }
        )]
    );
    assert!(kart.tnt_fuse().is_some());
    // The server relays it with our Wumpa.
    w.receive(Down::Wumpa(6));
    w.receive(Down::Hit {
        victim: ME,
        shooter: THEM,
        shot: 4,
        kind: HitKind::TntOnHead,
        wumpa: 6,
    });
    tick(&mut w, &world, &mut kart, 40);
    assert_eq!(w.wumpa(), 6);
    assert!(w.our_look().tnt.is_some());
    // Its fuse runs out on our head: a self-inflicted blast.
    tick(&mut w, &world, &mut kart, 90);
    assert!(w.outbox.contains(&(
        ME,
        Up::Hit {
            shooter: ME,
            shot: 0,
            kind: HitKind::Blasted
        }
    )));
}

#[test]
fn hits_on_others_wait_for_the_server() {
    let (mut w, world, mut kart) = setup();
    let them = ORIGIN + Vec3::X * 60.0;
    // From a kart's height off the ground, as a shot leaves a kart.
    w.receive(shot(ME, Item::Missile, 2, them + Vec3::new(0.0, 0.5, 30.0)));
    tick(&mut w, &world, &mut kart, 1);
    assert_eq!(w.shots().count(), 1);
    // Their client says they were hit: the missile ends here too, and they tumble.
    w.receive(Down::Hit {
        victim: THEM,
        shooter: ME,
        shot: 2,
        kind: HitKind::Blasted,
        wumpa: 0,
    });
    tick(&mut w, &world, &mut kart, 2);
    assert_eq!(w.shots().count(), 0);
    assert!(w.outbox.is_empty(), "never ours to report");
    let r = w.reaction(THEM).expect("tumbling");
    assert!(r.rotation.angle_between(Quat::IDENTITY) > 0.01 || r.scale != Vec3::ONE);
}

#[test]
fn a_second_press_fires_the_shield() {
    let (mut w, world, mut kart) = setup();
    let here = from_lu(kart.pose().position, ORIGIN);
    w.receive(Down::Item {
        item: Item::Shield,
        shots: 1,
    });
    tick(&mut w, &world, &mut kart, usize::from(ROLL_FRAMES) + 1);
    w.press(true, false);
    tick(&mut w, &world, &mut kart, 1);
    assert_eq!(w.outbox.len(), 1);
    w.outbox.clear();
    w.receive(shot(ME, Item::Shield, 1, here));
    tick(&mut w, &world, &mut kart, 2);
    assert!(kart.shield().is_some());
    // The second press sends `W Z`; the shield leaves our kart at once, the bomb on the relay.
    w.press(true, false);
    tick(&mut w, &world, &mut kart, 1);
    assert!(matches!(
        w.outbox[..],
        [(ME, Up::ShieldShot { shot: 2, .. })]
    ));
    assert!(kart.shield().is_none());
}

#[test]
fn a_second_press_detonates_our_bomb_on_the_relay() {
    let (mut w, world, mut kart) = setup();
    let here = from_lu(kart.pose().position, ORIGIN);
    w.receive(Down::Item {
        item: Item::Bomb,
        shots: 1,
    });
    tick(&mut w, &world, &mut kart, usize::from(ROLL_FRAMES) + 1);
    w.press(true, false);
    tick(&mut w, &world, &mut kart, 1);
    assert!(matches!(w.outbox[..], [(ME, Up::Fire { shot: 1, .. })]));
    w.outbox.clear();
    w.receive(shot(ME, Item::Bomb, 1, here));
    tick(&mut w, &world, &mut kart, 5);
    assert_eq!(w.shots().count(), 1, "rolling");
    // The second press sends `W D`; the bomb goes off when the server relays it.
    w.press(true, false);
    tick(&mut w, &world, &mut kart, 1);
    assert_eq!(std::mem::take(&mut w.outbox), vec![(ME, Up::Detonate)]);
    assert_eq!(w.shots().count(), 1, "nothing goes off before the relay");
    w.receive(Down::Detonate { shooter: ME });
    tick(&mut w, &world, &mut kart, 30);
    assert!(
        w.shots()
            .all(|(_, b, _, _)| !matches!(b, Body::Bomb(x) if !x.is_shield() && !x.done())),
        "the bomb went off"
    );
}

#[test]
fn positions_and_facings_survive_the_wire() {
    // lu from the session, Bevy world, WoW world at the wire's 2 decimals, and back.
    for lu in [
        Vec3::ZERO,
        Vec3::new(1234.5, -80.0, -9876.25),
        Vec3::new(-40.0, 300.0, 15.0),
    ] {
        let wow = bevy_to_wow(from_lu(lu, ORIGIN)).map(|v| (v * 100.0).round() / 100.0);
        let back = to_lu(wow_to_bevy(wow), ORIGIN);
        // Half a hundredth of a yard is under a fifth of a lu.
        assert!((back - lu).length() < 0.4, "{lu} → {back}");
    }
    for heading in [0, 1, 1024, 2047, 2048, 3000, 4095] {
        let yaw = (yaw_of(heading) * 1000.0).round() / 1000.0;
        assert_eq!(heading_of(yaw), heading, "{heading}");
    }
}

#[test]
fn ctr_hit_kinds_carry_over_to_the_kart() {
    for (kind, kart) in [
        (HitKind::SpinOut, ctr_kart::HitKind::SpinOut),
        (HitKind::Blasted, ctr_kart::HitKind::Blasted),
        (HitKind::Squished, ctr_kart::HitKind::Squished),
        (HitKind::TntOnHead, ctr_kart::HitKind::TntOnHead),
    ] {
        assert_eq!(kart_hit(kind), kart);
    }
}

#[test]
fn a_new_race_keeps_the_layout_that_came_with_its_roster() {
    let (mut w, world, mut kart) = setup();
    let layout = || Down::Layout {
        first: 0,
        crates: vec![CrateSpot {
            kind: CrateKind::Weapon,
            at: bevy_to_wow(ORIGIN + Vec3::Z * 50.0),
        }],
    };
    // The roster binds the race in the frame its layout arrives: a rebind keeps it.
    w.receive(layout());
    w.rebind();
    tick(&mut w, &world, &mut kart, 1);
    assert_eq!(w.crates().len(), 1, "the layout that came with the roster");

    // A race's end drops everything, waiting lines too.
    w.receive(layout());
    w.clear();
    tick(&mut w, &world, &mut kart, 1);
    assert!(w.crates().is_empty());
}

#[test]
fn a_rain_cloud_spins_our_item_until_the_servers_new_roll() {
    let (mut w, world, mut kart) = setup();
    w.receive(Down::Item {
        item: Item::Bomb3,
        shots: 3,
    });
    tick(&mut w, &world, &mut kart, usize::from(ROLL_FRAMES) + 2);
    assert_eq!(w.held(), HeldView::Item(Item::Bomb3));
    w.receive(Down::Cloud {
        victim: ME,
        effect: CloudEffect::ItemRoll,
        locked: true,
    });
    tick(&mut w, &world, &mut kart, 1);
    assert_eq!(w.held(), HeldView::Spinning);
    assert!(w.our_look().cloud);
    assert_eq!(kart.cloud(), Some(CloudEffect::ItemRoll));
    // Fire does nothing for the cloud's 240 frames, and the roulette waits past them.
    for _ in 0..CLOUD_FRAMES + 30 {
        w.press(true, false);
        tick(&mut w, &world, &mut kart, 1);
    }
    assert!(w.outbox.is_empty(), "{:?}", w.outbox);
    assert_eq!(w.held(), HeldView::Spinning);
    assert!(!w.our_look().cloud);
    assert_eq!(kart.cloud(), None);
    w.receive(Down::Item {
        item: Item::Turbo,
        shots: 1,
    });
    tick(&mut w, &world, &mut kart, 2);
    assert_eq!(w.held(), HeldView::Item(Item::Turbo));
    // A slow cloud on another racer only hangs over its kart.
    w.receive(Down::Cloud {
        victim: THEM,
        effect: CloudEffect::Slow,
        locked: false,
    });
    tick(&mut w, &world, &mut kart, 1);
    assert!(w.others().any(|(id, _, look)| id == THEM && look.cloud));
    assert_eq!(kart.cloud(), None);
}

#[test]
fn a_masked_kart_ramming_us_is_reported_once() {
    let (mut w, world, mut kart) = setup();
    let there = ORIGIN + Vec3::X * 60.0;
    w.receive(shot(THEM, Item::Mask, 1, there));
    tick(&mut w, &world, &mut kart, 1);
    let mut nearby = [crate::NearbyKart {
        id: THEM,
        ..Default::default()
    }];
    w.arm(&mut nearby);
    assert!(nearby[0].masked && !nearby[0].shielded && !nearby[0].turbo);
    let touching = ctr_kart::OtherKart {
        position: kart.pose().position + Vec3::Z * 60.0,
        weight: 256,
        id: THEM,
        masked: true,
        ..Default::default()
    };
    for _ in 0..5 {
        kart.events.clear();
        kart.tick_among(
            &world,
            &ctr_kart::Input::default(),
            &ctr_kart::Modifiers::default(),
            &[touching],
        );
        w.tick(&world, &mut kart, ORIGIN);
    }
    assert_eq!(
        std::mem::take(&mut w.outbox),
        vec![(
            ME,
            Up::Ram {
                attacker: THEM,
                ram: Ram::Mask
            }
        )]
    );
    // The relay sets our Wumpa; a shield ram relayed takes the attacker's shield off its ghost.
    w.receive(shot(THEM, Item::Shield, 2, there));
    tick(&mut w, &world, &mut kart, 1);
    w.arm(&mut nearby);
    assert!(nearby[0].shielded);
    w.receive(Down::Ram {
        victim: ME,
        attacker: THEM,
        ram: Ram::Shield,
        wumpa: 4,
    });
    tick(&mut w, &world, &mut kart, 1);
    w.arm(&mut nearby);
    assert!(!nearby[0].shielded);
    // A turbo relayed arms it for the item's 75 frames.
    w.receive(shot(THEM, Item::Turbo, 3, there));
    tick(&mut w, &world, &mut kart, 1);
    w.arm(&mut nearby);
    assert!(nearby[0].turbo);
    tick(&mut w, &world, &mut kart, 75);
    w.arm(&mut nearby);
    assert!(!nearby[0].turbo);
}
