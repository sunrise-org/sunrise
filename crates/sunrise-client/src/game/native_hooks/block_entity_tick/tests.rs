use super::*;
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};
use mod_api::{
    BlockEntityLoadAdditionalEvent, BlockEntityPart, BlockEntitySaveAdditionalEvent,
    BlockEntityTickEvent, BlockPart, Hooks, HostTransaction, LevelHost, ModEntry, Registrar,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use sunrise_client::{
    chunk::Chunk,
    containers::{ModBlockEntityFields, ModContainer},
};

const POS: Pos = (1, 64, 1);
const CHUNK: (i32, i32) = (0, 0);
const KIND: &str = "view_tick:subscribed";
const UNSUBSCRIBED: &str = "view_tick:unsubscribed";
const MACHINE: &str = "view_tick:machine";
const CONTAINER_MACHINE: &str = "view_tick:container_machine";
const UNSUBSCRIBED_MACHINE: &str = "view_tick:unsubscribed_machine";

fn row(kind: &str, value: i32) -> BlockEntity {
    BlockEntity::Mod(Box::new(EngineBlockEntity::new(
        kind.into(),
        ModBlockEntityFields {
            data: vec![value],
            ..Default::default()
        },
    )))
}

fn place_subject(game: &mut Game<'_>, kind: &str) -> BlockEntityRef {
    game.world
        .chunks
        .insert(CHUNK, Arc::new(Chunk::empty(0, 0)));
    let block_id = if kind == UNSUBSCRIBED {
        UNSUBSCRIBED_MACHINE
    } else {
        MACHINE
    };
    let block = game.pack.assets.registry.default_state(block_id).unwrap();
    game.world.set(POS.0, POS.1, POS.2, block).unwrap();
    game.world.block_entities.insert(POS, row(kind, 23));
    game.world.modified.clear();
    let level = game.level(game.dim.id()).unwrap();
    game.block_entity(level, [POS.0, POS.1, POS.2]).unwrap()
}

fn access<'a, 'g>(game: &'a mut Game<'g>, subject: BlockEntityRef) -> TickAccess<'a, 'g> {
    TickAccess { game, subject }
}

fn enable_block_ticks(game: &mut Game<'_>) {
    game.player = Some(
        sunrise_client::motion::PlayerMotion::new(
            game.motion,
            sunrise_core::geometry::Vec3::new(8.5, 65.0, 8.5),
        )
        .unwrap(),
    );
    game.update_chunk_tickets();
}

#[test]
#[ignore = "daily"]
fn retaking_the_view_checks_the_instance_and_state() {
    const NAME: &str = "game::native_hooks::block_entity_tick::tests::retaking_the_view_checks_the_instance_and_state";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_RETAKE", &["run"]) {
        return;
    }
    with_ticker(|game, effects| {
        // Enter through the real dispatched Part, then change the block through its Host.
        game.options.dedicated = Some(Vec::new());
        place_subject(game, KIND);
        enable_block_ticks(game);
        *effects.retake.lock().unwrap() = Retake::Requested;
        game.tick_block_entities_with(&mut ()).unwrap();
        assert_eq!(
            *effects.retake.lock().unwrap(),
            Retake::Observed {
                before: 23,
                after: None,
            }
        );

        // Raw writes can also leave the row present on an incompatible block, and
        // replacing it at the same position must not resurrect the original loan.
        let subject = place_subject(game, KIND);
        let unlit = game.world.block(POS.0, POS.1, POS.2);
        let lit = crate::game::native_hooks::with_property(
            game.pack.assets.registry,
            unlit,
            "lit",
            "true",
        )
        .unwrap();
        assert_ne!(lit, unlit);
        let unlit_api = unlit.try_into_api().unwrap();
        let lit_api = lit.try_into_api().unwrap();
        let glass = game
            .pack
            .assets
            .registry
            .default_state("minecraft:glass")
            .unwrap();
        let mut access = access(game, subject);
        let mut scope = BlockEntityTickScope::new(&mut access);
        assert_eq!(
            scope.with_view(|view| (view.data_slots()[0], view.state())),
            Some((23, unlit_api))
        );
        let host = scope.host();
        let level = subject.level(host).unwrap();
        let pos = subject.position(host).unwrap();
        access.game.world.set(pos[0], pos[1], pos[2], lit).unwrap();
        assert_eq!(
            BlockEntityTickScope::new(&mut access)
                .with_view(|view| (view.data_slots()[0], view.state())),
            Some((23, lit_api))
        );
        access
            .game
            .world
            .set(pos[0], pos[1], pos[2], glass)
            .unwrap();
        assert_eq!(access.game.block_entity(level, pos), Some(subject));
        assert_eq!(
            BlockEntityTickScope::new(&mut access).with_view(|_| ()),
            None
        );
        access.game.world.set(pos[0], pos[1], pos[2], lit).unwrap();
        assert_eq!(
            BlockEntityTickScope::new(&mut access).with_view(|view| view.state()),
            Some(lit_api)
        );
        access.game.world.block_entities.remove(&POS);
        assert_eq!(
            BlockEntityTickScope::new(&mut access).with_view(|_| ()),
            None
        );
        access.game.world.block_entities.insert(POS, row(KIND, 99));
        assert_eq!(
            BlockEntityTickScope::new(&mut access).with_view(|_| ()),
            None
        );
    });
}

#[test]
#[ignore = "daily"]
fn read_only_and_noop_loans_do_not_journal_or_dirty() {
    const NAME: &str = "game::native_hooks::block_entity_tick::tests::read_only_and_noop_loans_do_not_journal_or_dirty";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_NOOP", &["run"]) {
        return;
    }
    with_ticker(|game, _| {
        let subject = place_subject(game, KIND);
        let result: Result<(), ()> = game.transaction(|game, _| {
            let mut access = access(game, subject);
            BlockEntityTickScope::new(&mut access)
                .with_view(|mut view| {
                    assert_eq!(view.data_slots(), [23]);
                    assert!(!view.set_data_slot(0, 23));
                    assert!(!view.set_data_slot(1, 24));
                    assert!(!view.set_data_slots(&[23]));
                })
                .unwrap();
            assert_eq!(game.journal_kept(), (0, 0));
            Ok(())
        });
        assert_eq!(result, Ok(()));
        assert!(game.world.modified.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn a_scope_transaction_aborts_subject_and_dropped_item_edits_together() {
    const NAME: &str = "game::native_hooks::block_entity_tick::tests::a_scope_transaction_aborts_subject_and_dropped_item_edits_together";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_ABORT", &["run"]) {
        return;
    }
    with_ticker(|game, _| {
        let subject = place_subject(game, KIND);
        game.spawn_item(
            sunrise_client::items::ItemStack::new("minecraft:stone", 5),
            [1.0, 64.0, 1.0],
            [0.0; 3],
            0,
        );
        let drop_id = game.dropped[0].id;
        let item = game.item_entity(game.entity(drop_id).unwrap()).unwrap();
        let count = item.with_stack(game, |stack| stack.count()).unwrap();
        game.world.modified.clear();
        let mut access = access(game, subject);
        let mut scope = BlockEntityTickScope::new(&mut access);
        let result: Result<(), ()> = scope.transaction(|scope, _| {
            scope
                .with_view(|mut view| {
                    assert!(view.set_data_slot(0, 24));
                    assert!(view.set_nbt("recipe".into(), Some(Tag::Int(6))));
                })
                .ok_or(())?;
            let host = scope.host();
            let rest = item
                .with_stack(host, |stack| stack.project())
                .flatten()
                .unwrap()
                .shrink(1)
                .unwrap();
            assert!(item.set_stack(host, rest).unwrap());
            assert_eq!(
                item.with_stack(host, |stack| stack.count()),
                Some(count - 1)
            );
            scope
                .with_view(|view| assert_eq!(view.data_slots(), [24]))
                .ok_or(())?;
            Err(())
        });
        assert_eq!(result, Err(()));
        scope
            .with_view(|view| {
                assert_eq!(view.data_slots(), [23]);
                assert!(view.nbt().is_empty());
            })
            .unwrap();
        assert_eq!(
            item.with_stack(scope.host(), |stack| stack.count()),
            Some(count)
        );
    });
}

#[derive(Debug, Default, PartialEq, Eq)]
enum Retake {
    #[default]
    Off,
    Requested,
    Observed {
        before: i32,
        after: Option<i32>,
    },
}

#[derive(Clone, Default)]
struct Effects {
    notices: Arc<AtomicU32>,
    ticks: Arc<AtomicU32>,
    loaded_items: Arc<Mutex<Option<Tag>>>,
    saved_items: Arc<Mutex<Option<Tag>>>,
    retake: Arc<Mutex<Retake>>,
}
#[mod_api::sunrise_part]
impl BlockEntityPart for Effects {
    fn block_entity_ticks(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::BlockEntityTicksEvent<'_>,
    ) {
        event.ticks = true;
    }

    fn block_entity_tick(&self, event: &mut BlockEntityTickEvent<'_>) {
        self.ticks.fetch_add(1, Ordering::Relaxed);
        if *self.retake.lock().unwrap() == Retake::Requested {
            let (handle, before) = event
                .subject
                .with_view(|view| (view.handle(), view.data_slots()[0]))
                .unwrap();
            let host = event.subject.host();
            let level = handle.level(host).unwrap();
            let pos = handle.position(host).unwrap();
            let glass = host.block_state_id("minecraft:glass").unwrap();
            assert_eq!(
                level.set_block(host, pos, glass, mod_api::BlockUpdateFlags::NONE),
                Ok(true)
            );
            let after = event.subject.with_view(|view| view.data_slots()[0]);
            *self.retake.lock().unwrap() = Retake::Observed { before, after };
        } else {
            let _ = event.subject.with_view(|mut view| view.set_data_slot(0, 1));
        }
    }

    fn block_entity_load_additional(
        &self,
        _: &dyn LevelHost,
        event: &mut BlockEntityLoadAdditionalEvent<'_>,
    ) {
        *self.loaded_items.lock().unwrap() = event.tag.get(&"Items".into()).cloned();
    }

    fn block_entity_save_additional(
        &self,
        _: &dyn LevelHost,
        event: &mut BlockEntitySaveAdditionalEvent<'_>,
    ) {
        *self.saved_items.lock().unwrap() = event.tag.get(&"Items".into()).cloned();
    }
}
#[mod_api::sunrise_part]
impl BlockPart for Effects {
    fn contents_changed(&self, _: &mut dyn ServerHost, event: &mut mod_api::ContentsChangedEvent) {
        if event.pos == mod_api::BlockPos::new(POS.0, POS.1, POS.2) {
            self.notices.fetch_add(1, Ordering::Relaxed);
        }
    }
}
#[mod_api::__subscriptions]
impl Hooks for Effects {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let kind = r
            .block_entity_type("subscribed")
            .blocks(&[MACHINE, CONTAINER_MACHINE])
            .build();
        r.block_entity_part(&kind, self.clone()).unwrap();
        r.block_entity_type("unsubscribed")
            .blocks(&[UNSUBSCRIBED_MACHINE])
            .build();
        let container_machine = r.block_key(CONTAINER_MACHINE);
        r.block_container(
            &container_machine,
            mod_api::ModContainer {
                slots: 1,
                ..Default::default()
            },
        );
    }

    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        event
            .register(mod_api::Block::new(MACHINE, "minecraft:furnace").with_item(false))
            .unwrap();
        event
            .register(mod_api::Block::new(CONTAINER_MACHINE, "minecraft:furnace").with_item(false))
            .unwrap();
        event
            .register(mod_api::Block::new(UNSUBSCRIBED_MACHINE, "minecraft:stone").with_item(false))
            .unwrap();
    }
}

fn with_ticker(test: impl FnOnce(&mut Game<'_>, &Effects)) {
    let effects = Effects::default();
    let mods: mod_api::Mods = vec![ModEntry::new("view_tick", Box::new(effects.clone()))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = crate::game::native_entity_refs::tests::fixture_root();
        let root = crate::game::mods::content_root(&base, &Default::default()).unwrap();
        with_game_at(root.prepared().to_owned(), |game| test(game, &effects));
    });
}

#[test]
#[ignore = "daily"]
fn an_invalid_resident_block_entity_is_not_dispatched() {
    use crate::game::{
        block_entity_ticks::{BlockEntityTick, BlockEntityTickSlot},
        published_loops::LoopFlow,
    };

    struct Invalidate {
        glass: BlockStateId,
        visited: bool,
    }
    impl BlockEntityTickSlot for Invalidate {
        fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
            assert_eq!(tick.pos, POS);
            self.visited = true;
            // This before-slot's raw write keeps the row and its ticker live.
            tick.game
                .world
                .set(POS.0, POS.1, POS.2, self.glass)
                .unwrap();
            LoopFlow::Continue
        }
    }

    #[derive(Default)]
    struct ObserveAfter {
        visited: Vec<Pos>,
    }
    impl BlockEntityTickSlot for ObserveAfter {
        fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
            self.visited.push(tick.pos);
        }
    }

    const NAME: &str = "game::native_hooks::block_entity_tick::tests::an_invalid_resident_block_entity_is_not_dispatched";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_INVALID_TICK", &["run"]) {
        return;
    }
    with_ticker(|game, effects| {
        let subject = place_subject(game, KIND);
        let valid = game.world.block(POS.0, POS.1, POS.2);
        let glass = game
            .pack
            .assets
            .registry
            .default_state("minecraft:glass")
            .unwrap();
        enable_block_ticks(game);
        let mut slots = (
            Invalidate {
                glass,
                visited: false,
            },
            ObserveAfter::default(),
        );
        game.tick_block_entities_with(&mut slots).unwrap();
        assert!(slots.0.visited);
        assert!(slots.1.visited.is_empty());
        assert_eq!(effects.ticks.load(Ordering::Relaxed), 0);
        assert_eq!(subject.data_slots(game).unwrap(), [23]);

        game.world.set(POS.0, POS.1, POS.2, valid).unwrap();
        game.tick_block_entities_with(&mut slots.1).unwrap();
        assert_eq!(slots.1.visited, [POS]);
        assert_eq!(effects.ticks.load(Ordering::Relaxed), 1);
        assert_eq!(subject.data_slots(game).unwrap(), [1]);
    });
}

#[test]
#[ignore = "daily"]
fn invalid_resident_ticker_warns_once_until_a_valid_tick() {
    const NAME: &str = "game::native_hooks::block_entity_tick::tests::invalid_resident_ticker_warns_once_until_a_valid_tick";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_TICK_WARNING", &["run"]) {
        return;
    }

    struct Capture;
    static WARNINGS: AtomicU32 = AtomicU32::new(0);
    impl log::Log for Capture {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() == log::Level::Warn
                && metadata
                    .target()
                    .ends_with("::native_hooks::block_entity_tick")
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                WARNINGS.fetch_add(1, Ordering::Relaxed);
            }
        }
        fn flush(&self) {}
    }
    log::set_logger(&Capture).unwrap();
    log::set_max_level(log::LevelFilter::Warn);

    with_ticker(|game, effects| {
        place_subject(game, KIND);
        enable_block_ticks(game);
        let valid = game.world.block(POS.0, POS.1, POS.2);
        let glass = game
            .pack
            .assets
            .registry
            .default_state("minecraft:glass")
            .unwrap();
        // Bind the valid ticker before leaving an incompatible block under it.
        game.tick_block_entities_with(&mut ()).unwrap();
        for (state, warnings, ticks) in [
            (glass, 1, 1),
            (glass, 1, 1),
            (valid, 1, 2),
            (glass, 2, 2),
            (glass, 2, 2),
        ] {
            game.world.set(POS.0, POS.1, POS.2, state).unwrap();
            game.tick_block_entities_with(&mut ()).unwrap();
            assert_eq!(WARNINGS.load(Ordering::Relaxed), warnings);
            assert_eq!(effects.ticks.load(Ordering::Relaxed), ticks);
        }
    });
}

#[test]
#[ignore = "daily"]
fn ticker_and_handle_setters_require_explicit_set_changed() {
    const NAME: &str = "game::native_hooks::block_entity_tick::tests::ticker_and_handle_setters_require_explicit_set_changed";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_TICK", &["run"]) {
        return;
    }
    with_ticker(|game, effects| {
        let notices = &effects.notices;
        let subject = place_subject(game, KIND);
        let machine_state = game.world.block(POS.0, POS.1, POS.2);
        game.world.block_entities.set_mod_containers(
            [(
                machine_state.0,
                Arc::new(ModContainer {
                    slots: 1,
                    ..Default::default()
                }),
            )]
            .into_iter()
            .collect(),
        );
        game.world.block_log = Some(Vec::new());
        game.tick_mod_block_entity(POS);
        assert_eq!(subject.data_slots(game).unwrap(), [1]);
        assert!(!game.world.modified.contains(&CHUNK));
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
        assert_eq!(notices.load(Ordering::Relaxed), 0);

        let stack = sunrise_client::items::ItemStack::new("minecraft:stone", 2)
            .try_into_api()
            .unwrap();
        {
            let mut access = access(game, subject);
            BlockEntityTickScope::new(&mut access)
                .with_view(|mut view| {
                    assert!(view.set_data_slots(&[10, 20]));
                    assert!(view.set_data_slot(1, 21));
                    assert!(view.set_nbt("recipe".into(), Some(Tag::Int(7))));
                    assert_eq!(view.set_item_slot(0, Some(stack)), Ok(true));
                })
                .unwrap();
        }
        assert!(!game.world.modified.contains(&CHUNK));
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
        assert_eq!(notices.load(Ordering::Relaxed), 0);

        assert!(subject.set_data_slot(game, 0, 2).unwrap());
        assert_eq!(subject.set_data_slots(game, vec![3, 4]), Ok(true));
        assert_eq!(subject.data_slots(game).unwrap(), [3, 4]);
        assert!(!game.world.modified.contains(&CHUNK));
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
        assert_eq!(notices.load(Ordering::Relaxed), 0);

        subject.set_changed(game).unwrap();
        assert!(game.world.modified.contains(&CHUNK));
        assert_eq!(
            notices.load(Ordering::Relaxed),
            1,
            "explicit notice is wired"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_registered_type_without_a_tick_part_is_not_woken_or_changed() {
    const NAME: &str = "game::native_hooks::block_entity_tick::tests::a_registered_type_without_a_tick_part_is_not_woken_or_changed";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_NO_TICK", &["run"]) {
        return;
    }
    with_ticker(|game, effects| {
        let subject = place_subject(game, UNSUBSCRIBED);
        game.world.block_entities.sleep(POS);
        game.tick_mod_block_entity(POS);
        assert_eq!(subject.data_slots(game).unwrap(), [23]);
        assert!(game.world.block_entities.is_asleep(&POS));
        assert!(game.world.modified.is_empty());
        assert_eq!(effects.notices.load(Ordering::Relaxed), 0);
        assert_eq!(effects.ticks.load(Ordering::Relaxed), 0);
    });
}

#[test]
#[ignore = "daily"]
fn chunk_save_reloads_reordered_slotless_items() {
    use sunrise_client::save;

    const NAME: &str =
        "game::native_hooks::block_entity_tick::tests::chunk_save_reloads_reordered_slotless_items";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_ITEMS_ORDER", &["run"]) {
        return;
    }
    with_ticker(|game, effects| {
        let subject = place_subject(game, KIND);
        let item = |slot| {
            Tag::compound([
                ("Slot", Tag::Byte(slot)),
                ("id", Tag::String("minecraft:stone".into())),
                ("count", Tag::Int(1)),
            ])
        };
        let initial = Tag::List(vec![item(1), item(0)]);
        let reordered = Tag::List(vec![item(0), item(1)]);
        let write_items = |game: &mut Game<'_>, subject: BlockEntityRef, items: Tag| {
            let mut access = access(game, subject);
            let mut scope = BlockEntityTickScope::new(&mut access);
            assert_eq!(
                scope.with_view(|mut view| view.set_nbt("Items".into(), Some(items))),
                Some(true)
            );
            subject.set_changed(scope.host()).unwrap();
        };
        let save_reload = |game: &Game<'_>| {
            let snapshots = game.with_entities([game.world.chunks.get(&CHUNK).unwrap()]);
            let registry = game.pack.assets.registry;
            let biomes = game.res.biomes();
            let encoded = save::encode(&snapshots[0], registry, biomes, game.res.items());
            save::decode(&encoded, registry, biomes).unwrap()
        };

        write_items(game, subject, initial.clone());
        let mut loaded = save_reload(game);
        assert_eq!(effects.saved_items.lock().unwrap().as_ref(), Some(&initial));
        let BlockEntity::Mod(entity) = &loaded.block_entities[0].1 else {
            panic!("expected slotless mod block entity");
        };
        assert!(entity.items.is_empty());
        assert_eq!(entity.nbt.get(&"Items".into()), Some(&initial));
        game.world.install_chunk_block_entities(&mut loaded);
        game.world.chunks.insert(CHUNK, Arc::new(loaded));
        game.world.modified.clear();
        let pos = [POS.0, POS.1, POS.2];
        let subject = game
            .block_entity(game.level(game.dim.id()).unwrap(), pos)
            .unwrap();
        game.fire_pending_block_entity_loads();
        assert_eq!(
            effects.loaded_items.lock().unwrap().as_ref(),
            Some(&initial)
        );
        assert_eq!(
            subject.data(game).unwrap().get(&"Items".into()),
            Some(&initial)
        );
        // Capture a save straight after loading as well: the hook must not sort the rows.
        *effects.saved_items.lock().unwrap() = None;
        let _ = save_reload(game);
        assert_eq!(effects.saved_items.lock().unwrap().as_ref(), Some(&initial));
        write_items(game, subject, reordered.clone());

        let reloaded = save_reload(game);
        let BlockEntity::Mod(entity) = &reloaded.block_entities[0].1 else {
            panic!("expected slotless mod block entity");
        };
        assert!(entity.items.is_empty());
        assert_eq!(entity.nbt.get(&"Items".into()), Some(&reordered));
    });
}

#[test]
#[ignore = "daily"]
fn set_nbt_items_follows_declared_slot_count_and_clears_decoded_slots() {
    const NAME: &str = "game::native_hooks::block_entity_tick::tests::set_nbt_items_follows_declared_slot_count_and_clears_decoded_slots";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_ITEMS_NBT", &["run"]) {
        return;
    }
    with_ticker(|game, _| {
        let subject = place_subject(game, KIND);
        let machine_state = game.world.block(POS.0, POS.1, POS.2);

        // When the block declares item slots (slot_count > 0), "Items" is engine-owned.
        game.world.block_entities.set_mod_containers(
            [(
                machine_state.0,
                Arc::new(ModContainer {
                    slots: 1,
                    ..Default::default()
                }),
            )]
            .into_iter()
            .collect(),
        );
        {
            let mut access = access(game, subject);
            BlockEntityTickScope::new(&mut access)
                .with_view(|mut view| {
                    assert!(!view.set_nbt("Items".into(), Some(Tag::Int(1))));
                    assert!(view.nbt().get(&"Items".into()).is_none());
                })
                .unwrap();
        }

        // Without declared item slots (slot_count == 0), "Items" is ordinary NBT.
        // Seed disk-decoded `entity.items` to verify equality no-op and stale-slot clearing.
        game.world
            .block_entities
            .set_mod_containers(Default::default());
        let decoded = sunrise_client::items::ItemStack::new("minecraft:stone", 3);
        if let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&POS) {
            entity.items = vec![Some(decoded)];
        } else {
            panic!("mod entity missing");
        }
        let expected_decoded = Tag::List(vec![Tag::compound([
            ("Slot", Tag::Byte(0)),
            ("id", Tag::String("minecraft:stone".into())),
            ("count", Tag::Int(3)),
        ])]);

        // Writing the equal encoded value is a no-op and does not journal.
        let result: Result<(), ()> = game.transaction(|game, _| {
            let mut access = access(game, subject);
            BlockEntityTickScope::new(&mut access)
                .with_view(|mut view| {
                    assert!(!view.set_nbt("Items".into(), Some(expected_decoded.clone())));
                })
                .unwrap();
            assert_eq!(game.journal_kept(), (0, 0));
            Ok(())
        });
        assert_eq!(result, Ok(()));

        // Mutating "Items" clears stale decoded `entity.items` and stores the new NBT.
        let replacement = Tag::List(vec![Tag::compound([(
            "custom",
            Tag::String("payload".into()),
        )])]);
        {
            let mut access = access(game, subject);
            BlockEntityTickScope::new(&mut access)
                .with_view(|mut view| {
                    assert!(view.set_nbt("Items".into(), Some(replacement.clone())));
                    assert_eq!(view.nbt().get(&"Items".into()), Some(&replacement));
                    assert!(!view.set_nbt("Items".into(), Some(replacement.clone())));
                })
                .unwrap();
        }
        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&POS) else {
            panic!("mod entity missing");
        };
        assert!(
            entity.items.is_empty(),
            "stale decoded item slots must be cleared when slotless Items NBT changes"
        );
        let saved = sunrise_client::save::block_entity_encode::encode(
            POS,
            game.world.block_entities.get(&POS).unwrap(),
            MACHINE,
            game.res.items(),
        );
        assert_eq!(saved.get("Items"), Some(&replacement));

        // Clearing "Items" removes it from both NBT and encoded output.
        {
            let mut access = access(game, subject);
            BlockEntityTickScope::new(&mut access)
                .with_view(|mut view| {
                    assert!(view.set_nbt("Items".into(), None));
                    assert!(view.nbt().get(&"Items".into()).is_none());
                    assert!(!view.set_nbt("Items".into(), None));
                })
                .unwrap();
        }
        let saved = sunrise_client::save::block_entity_encode::encode(
            POS,
            game.world.block_entities.get(&POS).unwrap(),
            MACHINE,
            game.res.items(),
        );
        assert_eq!(saved.get("Items"), None);

        // Save/reload round-trip: canonical item-shaped "Items" stays verbatim in `nbt`
        // for slotless holders and is readable via `with_view`, while container-backed
        // holders still decode canonical entries into `entity.items`.
        {
            let mut access = access(game, subject);
            BlockEntityTickScope::new(&mut access)
                .with_view(|mut view| {
                    assert!(view.set_nbt("Items".into(), Some(expected_decoded.clone())));
                    assert_eq!(view.nbt().get(&"Items".into()), Some(&expected_decoded));
                })
                .unwrap();
        }
        let saved_canonical = sunrise_client::save::block_entity_encode::encode(
            POS,
            game.world.block_entities.get(&POS).unwrap(),
            MACHINE,
            game.res.items(),
        );
        assert_eq!(saved_canonical.get("Items"), Some(&expected_decoded));

        let (reloaded_pos, reloaded) =
            sunrise_client::save::decode_block_entity_for_block(&saved_canonical, MACHINE)
                .expect("slotless entity reloads");
        assert_eq!(reloaded_pos, POS);
        let BlockEntity::Mod(reloaded_mod) = &reloaded else {
            panic!("expected mod block entity");
        };
        assert!(
            reloaded_mod.items.is_empty(),
            "slotless load must keep canonical Items in nbt, not entity.items"
        );
        assert_eq!(
            reloaded_mod.nbt.get(&"Items".into()),
            Some(&expected_decoded)
        );
        game.world.block_entities.insert(POS, reloaded);
        let level = game.level(game.dim.id()).unwrap();
        let reloaded_subject = game.block_entity(level, [POS.0, POS.1, POS.2]).unwrap();
        {
            let mut access = access(game, reloaded_subject);
            BlockEntityTickScope::new(&mut access)
                .with_view(|mut view| {
                    assert_eq!(view.nbt().get(&"Items".into()), Some(&expected_decoded));
                    assert!(!view.set_nbt("Items".into(), Some(expected_decoded.clone())));
                })
                .unwrap();
        }

        let (_, container_reloaded) = sunrise_client::save::decode_block_entity_for_block(
            &saved_canonical,
            CONTAINER_MACHINE,
        )
        .expect("container-backed entity reloads");
        let BlockEntity::Mod(container_mod) = &container_reloaded else {
            panic!("expected mod block entity");
        };
        assert_eq!(
            container_mod.items,
            vec![Some(sunrise_client::items::ItemStack::new(
                "minecraft:stone",
                3
            ))]
        );
        assert!(container_mod.nbt.get(&"Items".into()).is_none());
    });
}
