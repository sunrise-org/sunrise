//! Native Parts exercise tick replies against real Game storage.
use mod_api::{Hooks, Registrar};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use sunrise_client::{
    chunk::Chunk,
    containers::{BlockEntity, ModBlockEntity},
};
#[path = "block_entity_tick_reply_tests/fixture.rs"]
mod fixture;

struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        fixture::register(registrar);
    }
}

struct Effects(Arc<[AtomicU32; 2]>);
#[mod_api::sunrise_part]
impl mod_api::BlockPart for Effects {
    fn contents_changed(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ContentsChangedEvent,
    ) {
        if event.pos == sunrise_core::position::BlockPos::new(1, 64, 1) {
            self.0[0].fetch_add(1, Ordering::Relaxed);
        }
    }

    fn on_neighbor_change(
        &self,
        _: &dyn mod_api::LevelHost,
        event: &mut mod_api::OnNeighborChangeEvent,
    ) {
        if event.pos == sunrise_core::position::BlockPos::new(1, 64, 2)
            && event.neighbor == sunrise_core::position::BlockPos::new(1, 64, 1)
        {
            self.0[1].fetch_add(1, Ordering::Relaxed);
        }
    }
}
#[mod_api::__subscriptions]
impl Hooks for Effects {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let stone = registrar.block_key("minecraft:stone");
        registrar.block_part(stone, Effects(self.0.clone()));
        let glass = registrar.block_key("minecraft:glass");
        registrar.block_part(glass, Effects(self.0.clone()));
    }
}

#[test]
#[ignore = "daily"]
fn native_tick_reply_matrix() {
    const NAME: &str = "game::block_entity_tick_reply_tests::native_tick_reply_matrix";
    if super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "SUNRISE_TICK_REPLY_CHILD",
        &["game"],
    ) {
        return;
    }
    sunrise_guest_stack::run_on_guest_stack(|| {
        let mut entries = vec![mod_api::ModEntry::new(
            "tick_reply_fixture",
            Box::new(Native),
        )];
        let effects = Arc::new([AtomicU32::new(0), AtomicU32::new(0)]);
        entries.push(mod_api::ModEntry::new(
            "tick_effects",
            Box::new(Effects(effects.clone())),
        ));
        let mods: mod_api::Mods = entries.into();
        mod_platform::testing::with_mods(mods.clone(), || {
            super::api_values::install_parts(&mods).unwrap();
            mod_api::__private::mark_game_thread();
            let base =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let root = super::mods::content_root(&base, &Default::default()).unwrap();
            super::native_entity_refs::tests::with_game_at(root.prepared().to_owned(), |game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for x in [1, 2] {
                    game.world.set(x, 64, 1, stone).unwrap();
                }
                let glass = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:glass")
                    .unwrap();
                game.world.set(1, 64, 2, glass).unwrap();
                let state = game.world.block(1, 64, 1).0;
                game.world.block_entities.set_mod_containers(
                    [(
                        state,
                        Arc::new(sunrise_client::containers::ModContainer {
                            slots: 1,
                            ..Default::default()
                        }),
                    )]
                    .into_iter()
                    .collect(),
                );
                // The type is registered, and other types do subscribe. Neither
                // a lazy container nor a slotless holder should be normalized here.
                for (pos, items) in [
                    ((1, 64, 1), vec![]),
                    (
                        (1, 64, 2),
                        vec![Some(sunrise_client::items::ItemStack::new(
                            "minecraft:stone",
                            1,
                        ))],
                    ),
                ] {
                    let data = sunrise_client::containers::ModBlockEntityFields {
                        data: vec![23],
                        items: items.clone(),
                        ..Default::default()
                    };
                    let entity = ModBlockEntity::new("tick_reply_fixture:no_ticker".into(), data);
                    assert!(entity.numeric_id().is_some());
                    let revision = entity.revision();
                    game.world
                        .block_entities
                        .insert(pos, BlockEntity::Mod(Box::new(entity)));
                    let generation = game.world.block_entities.generation(&pos);
                    game.world.block_entities.sleep(pos);
                    game.world.modified.clear();
                    game.tick_mod_block_entity(pos);
                    let Some(BlockEntity::Mod(current)) = game.world.block_entities.get(&pos)
                    else {
                        panic!("unsubscribed entity lost")
                    };
                    assert_eq!(current.id(), "tick_reply_fixture:no_ticker");
                    assert_eq!(current.data, [23]);
                    assert!(current.nbt.is_empty());
                    assert_eq!(current.items, items);
                    assert_eq!(current.revision(), revision);
                    assert_eq!(game.world.block_entities.generation(&pos), generation);
                    assert!(game.world.block_entities.is_asleep(&pos));
                    assert!(game.world.modified.is_empty());
                    assert!(
                        effects
                            .iter()
                            .all(|counter| counter.load(Ordering::Relaxed) == 0)
                    );
                    game.world.block_entities.remove(&pos);
                }
                game.spawn_item(
                    sunrise_client::items::ItemStack::new("minecraft:stone", 5),
                    [0.0; 3],
                    [0.0; 3],
                    0,
                );
                let item_id = game.dropped[0].id;
                let entity = |mode| {
                    ModBlockEntity::new(
                        "tick_reply_fixture:tick_reply".into(),
                        sunrise_client::containers::ModBlockEntityFields {
                            data: vec![mode, 0, 0, item_id as i32],
                            ..Default::default()
                        },
                    )
                };
                game.world
                    .block_entities
                    .insert((2, 64, 1), BlockEntity::Mod(Box::new(entity(0))));
                let mut expected_item_count = 5;
                for mode in [0, 8, 1, 4, 5, 9, 6, 7, 10, 3, 2] {
                    if mode == 7 {
                        expected_item_count = 2;
                    }
                    let original = entity(mode);
                    let revision = original.revision();
                    game.world
                        .block_entities
                        .insert((1, 64, 1), BlockEntity::Mod(Box::new(original)));
                    game.world.modified.clear();
                    let rounds = if matches!(mode, 0 | 8) { 64 } else { 1 };
                    for _ in 0..rounds {
                        for counter in effects.iter() {
                            counter.store(0, Ordering::Relaxed);
                        }
                        game.tick_mod_block_entity((1, 64, 1));
                        if mode == 2 {
                            assert!(matches!(
                                game.world.block_entities.get(&(1, 64, 1)),
                                Some(BlockEntity::Chest(_))
                            ));
                            assert!(game.world.modified.contains(&(0, 0)));
                            continue;
                        }
                        let Some(BlockEntity::Mod(current)) =
                            game.world.block_entities.get(&(1, 64, 1))
                        else {
                            panic!("entity lost")
                        };
                        let changed = matches!(mode, 1 | 4 | 5 | 9 | 10);
                        assert_eq!(
                            current.data,
                            [
                                mode,
                                match mode {
                                    1 => 1,
                                    5 => 7,
                                    _ => 0,
                                },
                                if matches!(mode, 4 | 9) { 42 } else { 0 },
                                item_id as i32,
                            ]
                        );
                        assert_eq!(
                            current.nbt,
                            if mode == 5 {
                                mod_api::Compound::from([("first".into(), mod_api::Tag::Int(7))])
                            } else {
                                Default::default()
                            }
                        );
                        assert_eq!(current.items.len(), usize::from(changed));
                        if mode == 10 {
                            assert_eq!(current.items[0].as_ref().unwrap().count, 2);
                        } else {
                            assert!(current.items.iter().all(Option::is_none));
                        }
                        assert_eq!(current.revision().identity(), revision.identity());
                        if changed {
                            assert!(current.revision().changes() > revision.changes());
                        } else {
                            assert_eq!(current.revision().changes(), revision.changes());
                        }
                        // Only the ticked entity's own change marks the chunk: mode 3's data-slot write to
                        // another block entity is silent until its mod notifies (`set_changed`).
                        assert_eq!(game.world.modified.contains(&(0, 0)), changed);
                        for counter in effects.iter() {
                            assert_eq!(counter.load(Ordering::Relaxed), u32::from(mode == 10));
                        }
                        assert_eq!(game.dropped[0].stack.count, expected_item_count);
                    }
                    if mode == 3 {
                        let Some(BlockEntity::Mod(sink)) =
                            game.world.block_entities.get(&(2, 64, 1))
                        else {
                            panic!("sink lost")
                        };
                        assert_eq!(sink.data, [0, 1, 0, item_id as i32]);
                        assert_eq!(sink.revision().changes(), 1);
                    }
                }
            });
        });
    });
}
