//! A mob's stab settled at its `Pause::Stab`: each event goes to the player it acts on, in the
//! order `KineticWeapon.damageEntities` raised it, and the tick's player selection is refreshed
//! before it resumes.
use super::*;

fn stab_on(target: Who) -> MobEvent {
    use sunrise_sim::mobs::{Damage, WeaponAttack, WeaponHit};
    MobEvent::WeaponAttack(WeaponHit {
        target,
        source: Damage::mob(71, [0.5, 80.0, 0.5]),
        weapon: sunrise_items::items::ItemStack::new("minecraft:iron_spear", 1),
        amount: 4.0,
        knockback: 0.0,
        direction: [0.0, 1.0],
        attack: WeaponAttack::Stab {
            deals_damage: true,
            deals_knockback: true,
            dismounts: false,
        },
    })
}

fn stab_memory(target: Who, contact: usize) -> MobEvent {
    MobEvent::StabMemory {
        attacker: 71,
        target,
        contact,
        attempted_damage: true,
        otherwise_affected: true,
    }
}

fn delivered(deliveries: Vec<StabDelivery>) -> Vec<String> {
    deliveries
        .into_iter()
        .map(|delivery| {
            let (to, event) = match delivery {
                StabDelivery::Host(event) => ("host".to_owned(), event),
                StabDelivery::Guest(conn, mob, event) => (format!("guest {conn} by {mob}"), event),
            };
            let what = match event {
                MobEvent::WeaponAttack(hit) => format!("stab {:?}", hit.target),
                MobEvent::StabMemory {
                    target, contact, ..
                } => format!("memory {target:?} {contact}"),
                MobEvent::EntityEvent { event, .. } => format!("event {event}"),
                other => format!("{other:?}"),
            };
            format!("{to}: {what}")
        })
        .collect()
}

/// `KineticWeapon.damageEntities` finishes each contact's `stabAttack` before the next and
/// broadcasts event 2 last: a guest met first (tagged with its connection, as
/// `MobWorld::route_player_event` tags it) is settled before the mob met after it, not after it.
/// An untagged player stab goes to the guest the tick sees, or to the host when it sees none.
#[test]
fn a_stab_settles_a_guest_contact_before_a_later_mob_contact() {
    let tagged = |event| MobEvent::ForPlayer {
        conn: Some(12),
        event: Box::new(event),
    };
    let broadcast = MobEvent::EntityEvent {
        mob: 71,
        event: 2,
        at: [0.5, 80.0, 0.5],
    };
    let stabs = vec![
        tagged(stab_on(Who::Player)),
        tagged(stab_memory(Who::Player, 0)),
        stab_on(Who::Mob(72)),
        stab_memory(Who::Mob(72), 1),
        broadcast.clone(),
    ];
    assert_eq!(
        delivered(route_stab_events(stabs, None, &[(12, 12)], 71)),
        [
            "guest 12 by 71: stab Player",
            "guest 12 by 71: memory Player 0",
            "host: stab Mob(72)",
            "host: memory Mob(72) 1",
            "host: event 2",
        ]
    );
    for (seen_guest, to) in [(Some(13), "guest 13 by 71"), (None, "host")] {
        let stabs = vec![
            stab_on(Who::Player),
            stab_on(Who::Mob(72)),
            broadcast.clone(),
        ];
        assert_eq!(
            delivered(route_stab_events(stabs, seen_guest, &[], 71)),
            [
                format!("{to}: stab Player"),
                "host: stab Mob(72)".to_owned(),
                "host: event 2".to_owned(),
            ]
        );
    }
}

fn player_view(x: f64, uuid: i32) -> PlayerView {
    PlayerView {
        position: Vec3::new(x, 80.0, 0.0),
        bounds: sunrise_core::geometry::Aabb::new(
            Vec3::new(x - 0.3, 80.0, -0.3),
            Vec3::new(x + 0.3, 81.8, 0.3),
        ),
        eye: [x, 81.62, 0.0],
        alive: true,
        invulnerable: false,
        creative: false,
        sneaking: false,
        swimming: false,
        look: [0.0; 3],
        main_hand: None,
        off_hand: None,
        head: None,
        chest: None,
        legs: None,
        feet: None,
        armor_cover: 0.0,
        invisible: false,
        spectator: false,
        gold_armor: false,
        hurt_by: None,
        last_hurt: None,
        velocity: Vec3::ZERO,
        yaw: 0.0,
        sprinting: false,
        riding: false,
        hero: false,
        effects: Vec::new(),
        uuid: [uuid, 0, 0, 0],
        allies: Default::default(),
        remote: true,
        sleeping: false,
        sleep_timer: 0,
        passenger: false,
        riding_vehicle: None,
        vehicle: None,
    }
}

/// A stab that kills the guest the tick sees: the pause's callback ends the views the tick held,
/// so its saved player selection is refreshed by identity before it resumes, as after a hurt or
/// an equipment event. A dead guest has no live view (`lan_guest_views` leaves it out) and none
/// is lent to the resumed tick, so the selection drops it; the host and another guest still in
/// play keep their places with their live views. No equipment pause comes between: the stab's
/// own refresh does it.
#[test]
fn a_stab_pause_drops_the_selected_guest_its_stab_killed() {
    assert!(refreshes_selection(Some(&Pause::Stab { events: 3 })));
    assert!(refreshes_selection(Some(&Pause::Equipment {
        first: sunrise_core::equipment_slot::EquipmentSlot::MainHand,
    })));
    assert!(!refreshes_selection(None));
    let host = player_view(1.0, 1);
    let tick_player = |entity_id, connection: Option<u32>, view: &PlayerView| TickPlayer {
        entity_id,
        connection,
        view: view.clone(),
    };
    let mut selection = PlayerSelection::new(
        vec![
            tick_player(1, None, &host),
            tick_player(12, Some(12), &player_view(12.0, 12)),
            tick_player(13, Some(13), &player_view(13.0, 13)),
        ],
        Some(12),
    )
    .unwrap();
    let identity = |conn: u32| Some(([conn as i32, 0, 0, 0], conn));
    let host_seat = (Some(&host), Some(([1, 0, 0, 0], 1)));
    // The guest the tick sees (12) is dead after the stab: only guest 13 has a view.
    let live = vec![(13, player_view(14.0, 13))];
    let slots = [(13, 0)].into_iter().collect();
    selection.refresh_views(|player| {
        live_selection_view(
            player,
            (Some(12), None, identity(12)),
            &live,
            &slots,
            identity,
            host_seat,
        )
    });
    assert!(selection.selected().is_none(), "the dead guest is dropped");
    assert!(selection.get(12).is_none());
    assert_eq!(selection.get(13).unwrap().view.position.x, 14.0);
    assert_eq!(selection.get(1).unwrap().view.position.x, 1.0);
    // A tick that sees the host: the host's view is the one lent to it. A guest whose
    // connection now names another entity (a rejoin) is not the player saved.
    let rejoined = |conn: u32| Some(([conn as i32, 0, 0, 0], conn + 100));
    selection.refresh_views(|player| {
        live_selection_view(
            player,
            (None, host_seat.0, host_seat.1),
            &live,
            &slots,
            rejoined,
            (None, None),
        )
    });
    assert!(selection.get(13).is_none());
    assert_eq!(selection.get(1).unwrap().view.position.x, 1.0);
}
