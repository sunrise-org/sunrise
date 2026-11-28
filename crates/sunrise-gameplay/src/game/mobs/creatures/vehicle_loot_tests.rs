//! `EntityPredicate`'s `vehicle` in a death's loot: a mob riding a boat or a minecart,
//! not only a mob, with the vehicle's type and fire state (`#5239`).
//! Each fixture loads prepared resources in its own process to isolate save metadata.
use super::*;
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
use sunrise_gameplay_common::game::boats_common::Boat;
use sunrise_world::loot::{EntityLootParams, LootData, PackOverlay};

const VEHICLE_LOOT_CHILD: &str = "SUNRISE_TEST_VEHICLE_LOOT_CHILD";

/// The table the death path rolls: one pool per vehicle fact, so the drop names which
/// predicate held.
const TABLE: &str = r#"{"pools":[
    {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}],
     "condition":{"type":"minecraft:entity_properties","entity":"this",
        "predicate":{"minecraft:vehicle":{"minecraft:entity_type":"minecraft:oak_boat"}}}},
    {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:gold_ingot"}],
     "condition":{"type":"minecraft:entity_properties","entity":"this",
        "predicate":{"minecraft:vehicle":{"minecraft:entity_type":"minecraft:birch_boat"}}}},
    {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:emerald"}],
     "condition":{"type":"minecraft:entity_properties","entity":"this",
        "predicate":{"minecraft:vehicle":{"minecraft:entity_type":"minecraft:oak_boat","minecraft:flags":{"is_on_fire":true}}}}}
]}"#;

/// The item entities a real death spawns, by item id.
fn death_drops(game: &mut ServerGame<'_>, victim: &Mob) -> Vec<String> {
    game.common.dropped.clear();
    game.mob_die(victim);
    game.common
        .dropped
        .iter()
        .map(|item| item.stack.item.to_string())
        .collect()
}

/// A zombie riding a boat reaches the `vehicle` predicate with the boat's type and no
/// `living` flag; a burning boat reaches `is_on_fire`; a boat the predicate names instead
/// of the ridden one keeps its own pool, the negative control.
#[test]
#[ignore = "daily"]
fn a_boat_mount_reaches_the_death_loot_vehicle_predicate() {
    if run_fresh_cases(
        "game::server_side::creatures_server::vehicle_loot_tests::a_boat_mount_reaches_the_death_loot_vehicle_predicate",
        VEHICLE_LOOT_CHILD,
        &["death_drops"],
    ) {
        return;
    }
    // The platform's init array only installs under `deps/`; a test binary elsewhere in
    // the target tree (and `--exact` runs) needs it explicitly, as `install` is idempotent.
    mod_platform::__private::test_platform::install();
    let files = vec![
        (
            "loot_table/test:vehicle".to_owned(),
            vec![TABLE.as_bytes().to_vec()],
        ),
        // The death path rolls the mob type's own table.
        (
            "loot_table/minecraft:entities/zombie".to_owned(),
            vec![TABLE.as_bytes().to_vec()],
        ),
    ];
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("items.json"), "{}").unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let loot = LootData::load(root.path(), &PackOverlay::new(&files), &defaults).unwrap();

    with_game(|game| {
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.server
            .server_loot
            .0
            .set(Some(std::sync::Arc::new(loot)));
        game.boats
            .push(Boat::new(70, "oak".into(), Vec3::ZERO, 0.0));
        game.boats
            .push(Boat::new(72, "birch".into(), Vec3::ZERO, 0.0));
        let mut victim = Mob::new(71, zombie, Vec3::ZERO, 0.0);
        victim.extra.mount = Some(70);
        victim.last_damage = Some((sunrise_sim::mobs::Damage::fall(), 0));
        game.mobs.push(victim.clone());

        let projected = game.target_loot_entity(crate::game::commands::Target::Mob(71));
        let vehicle = projected.vehicle.as_deref().expect("the boat mount");
        assert_eq!(vehicle.kind, "minecraft:oak_boat");
        assert!(!vehicle.living, "a boat is not a living entity");
        assert!(!vehicle.on_fire);
        assert_eq!(death_drops(game, &victim), ["minecraft:diamond"]);

        game.boats.update(0, |boat| boat.fire_ticks = 1);
        let projected = game.target_loot_entity(crate::game::commands::Target::Mob(71));
        assert!(projected.vehicle.as_deref().unwrap().on_fire);
        assert_eq!(
            death_drops(game, &victim),
            ["minecraft:diamond", "minecraft:emerald"],
            "the fire fact reaches the vehicle predicate"
        );

        victim.extra.mount = Some(72);
        game.mobs.update(0, |mob| mob.extra.mount = Some(72));
        let projected = game.target_loot_entity(crate::game::commands::Target::Mob(71));
        assert_eq!(
            projected.vehicle.as_deref().unwrap().kind,
            "minecraft:birch_boat"
        );
        assert_eq!(
            death_drops(game, &victim),
            ["minecraft:gold_ingot"],
            "the oak predicates do not match a birch boat"
        );
    });
}

/// The projection is what `EntityPredicate` reads, so a boat mount also answers a plain
/// entity roll (not only the death table): both go through `EntityLootParams`'s `this`.
#[test]
#[ignore = "daily"]
fn the_target_projection_is_the_entity_the_roll_reads() {
    if run_fresh_cases(
        "game::server_side::creatures_server::vehicle_loot_tests::the_target_projection_is_the_entity_the_roll_reads",
        VEHICLE_LOOT_CHILD,
        &["target_projection"],
    ) {
        return;
    }
    // The platform's init array only installs under `deps/`; a test binary elsewhere in
    // the target tree (and `--exact` runs) needs it explicitly, as `install` is idempotent.
    mod_platform::__private::test_platform::install();
    let files = vec![(
        "loot_table/test:vehicle".to_owned(),
        vec![TABLE.as_bytes().to_vec()],
    )];
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("items.json"), "{}").unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let loot = LootData::load(root.path(), &PackOverlay::new(&files), &defaults).unwrap();

    with_game(|game| {
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.boats
            .push(Boat::new(70, "oak".into(), Vec3::ZERO, 0.0));
        let mut victim = Mob::new(71, zombie, Vec3::ZERO, 0.0);
        victim.extra.mount = Some(70);
        game.mobs.push(victim);

        let this = game.target_loot_entity(crate::game::commands::Target::Mob(71));
        let params = EntityLootParams {
            this,
            ..EntityLootParams::default()
        };
        let mut random = sunrise_core::random::LegacyRandom::new(1);
        let items: Vec<String> = loot
            .roll_entity("test:vehicle", &mut random, &params)
            .into_iter()
            .map(|stack| stack.item.to_string())
            .collect();
        assert_eq!(items, ["minecraft:diamond"]);
    });
}
