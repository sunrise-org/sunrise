use crate::prepared_test_root::prepared_root;
use sunrise_core::geometry::Vec3;
use sunrise_core::prepared_test_support::STREAMING_INPUTS;
use sunrise_sim::mobs::{
    Mob, MobEvent, WeaponAttack, WeaponHit, Who, ai, riders,
    sensor_scan_tests::kind,
    spear_use::{main_hand_kinetic, start_using_item},
    world::with_resource_test_world,
};

/// What a mob world reads beyond the streaming tables (`MobTypes::load`, `MobData::load`,
/// `MotionResources::load`, `collisions::load`, `Items::load`).
const MOB_WORLD_INPUTS: &[&str] = &[
    "entity-types.json",
    "attributes.json",
    "mob-data.json",
    "movement-properties.nbt",
    "cached-collision.nbt",
    "gameplay-facts.json",
    "items.json",
];

fn spear_zombie() -> Mob {
    let mut m = Mob::new(1, kind(), Vec3::ZERO, 0.0);
    m.extra.set_slot_id(
        sunrise_items::equipment::MAINHAND,
        Some("minecraft:iron_spear".into()),
    );
    m
}

/// `LivingEntity.tick` stabs in `updatingUsingItem`, before `aiStep` moves the attacker: a
/// tick that stabs stops there unmoved (`Pause::Stab`), counting only the item use's events
/// (not the base tick's before them), and its resume moves it; a resume after settling the
/// stab removed the attacker runs none of the rest; a hostless tick runs through with the
/// stab in its events, and a tick without one does not stop.
#[test]
#[ignore = "daily"]
fn a_mob_tick_stops_at_its_stabs_before_the_mob_moves() {
    let root = prepared_root(&[STREAMING_INPUTS, MOB_WORLD_INPUTS]);
    let target = Mob::new(2, kind(), Vec3::new(0.0, 80.0, 1.6), 0.0);
    let others = [target.seen(None, &sunrise_sim::items::Items::empty())];
    let charging = || {
        let mut m = spear_zombie();
        assert!(start_using_item(&mut m, false));
        m.extra.using_item = main_hand_kinetic(&m).expect("an iron spear").delay_ticks + 1;
        m.position = Vec3::new(0.0, 80.0, 0.0);
        // Last tick it ran 0.1 blocks toward the target (`Entity.lastKnownSpeed`).
        m.previous = Vec3::new(0.0, 80.0, -0.1);
        m.velocity = Vec3::new(0.0, 0.0, 0.1);
        m
    };
    let stabs = |events: &[MobEvent]| {
        events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    MobEvent::WeaponAttack(WeaponHit {
                        target: Who::Mob(2),
                        attack: WeaponAttack::Stab { .. },
                        ..
                    })
                )
            })
            .count()
    };
    assert!(with_resource_test_world(
        &root,
        &others,
        true,
        true,
        |_, _| {},
        |w, _| {
            let start = Vec3::new(0.0, 80.0, 0.0);

            let mut m = charging();
            // A sound the base tick delivers (`Entity.baseTick`), before the item use.
            let earlier = MobEvent::Sound("minecraft:entity.zombie.hurt", [0.0; 3], 1.0, 1.0);
            m.outbox.push(earlier.clone());
            let mut level = sunrise_sim::health::TestLevel::new(0);
            let mut events = Vec::new();
            // The stab, its memory and event 2: the base tick's sound is not one of them.
            assert_eq!(
                ai::tick_until_stab(&mut m, w, &mut level.level(), &mut events),
                ai::UntilStab::Stabbed { events: 3 }
            );
            assert_eq!(events.len(), 4);
            assert!(
                matches!(&events[0], MobEvent::Sound(id, ..) if *id == "minecraft:entity.zombie.hurt")
            );
            assert_eq!(stabs(&events[1..]), 1);
            assert!(matches!(
                events[1..],
                [
                    MobEvent::WeaponAttack(_),
                    MobEvent::StabMemory { attacker: 1, .. },
                    MobEvent::EntityEvent {
                        mob: 1,
                        event: 2,
                        ..
                    }
                ]
            ));
            assert_eq!((m.position, m.velocity), (start, Vec3::new(0.0, 0.0, 0.1)));
            let paused = m.clone();
            assert!(ai::resume_hostless_after_stab(
                &mut m,
                w,
                &mut level.level(),
                &mut events
            ));
            assert_eq!(stabs(&events), 1);
            assert_ne!(m.position, start);

            // Settling the stab discarded the attacker (an `EntityMount` callback of a target
            // it dismounted can): its tick ends without moving, alone or with a vehicle loan.
            let mut m = paused.clone();
            m.removed = true;
            let mut events = Vec::new();
            assert!(!ai::resume_hostless_after_stab(
                &mut m,
                w,
                &mut level.level(),
                &mut events
            ));
            assert_eq!((m.position, m.velocity), (start, Vec3::new(0.0, 0.0, 0.1)));
            assert!(events.is_empty());
            let mut mobs = sunrise_sim::join_seq::JoinList::default();
            mobs.push(m);
            let mut movement = None;
            assert!(matches!(
                riders::resume_after_stab(
                    &mut mobs,
                    0,
                    w,
                    &mut level.level(),
                    &mut events,
                    &mut movement,
                ),
                ai::TickProgress::Complete(false)
            ));
            assert_eq!(mobs[0].position, start);
            assert!(events.is_empty());

            let mut m = charging();
            let mut events = Vec::new();
            assert!(ai::tick(&mut m, w, &mut level.level(), &mut events));
            assert_eq!(stabs(&events), 1);
            assert_ne!(m.position, start);

            // Not yet past the spear's delay: nothing to stab.
            let mut m = charging();
            m.extra.using_item = 1;
            let mut events = Vec::new();
            assert_eq!(
                ai::tick_until_stab(&mut m, w, &mut level.level(), &mut events),
                ai::UntilStab::Complete(true)
            );
            assert_eq!(stabs(&events), 0);
            assert_ne!(m.position, start);
        },
    ));
}
