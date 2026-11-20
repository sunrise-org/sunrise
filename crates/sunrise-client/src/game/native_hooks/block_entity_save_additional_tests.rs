#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{BlockEntityPart, BlockEntitySaveAdditionalEvent, Hooks, LevelHost, ModEntry};
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::{Arc, Mutex};
    use sunrise_client::{
        chunk::{Chunk, LevelShape},
        containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields, SHAPES_KEY},
        save::SaveDir,
    };
    use sunrise_core::{
        nbt::{Compound, JavaString, Limits, Tag, read_named},
        position::ChunkPos,
    };
    use sunrise_mod_runtime::attachments::ATTACHMENTS_KEY;
    use sunrise_world::region::RegionFile;

    thread_local! {
        static SAVED_AT: std::cell::RefCell<Vec<([i32; 3], mod_api::LevelRef)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    struct Counter(Arc<AtomicI32>);
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for Counter {}
    #[mod_api::sunrise_part]
    impl BlockEntityPart for Counter {
        fn block_entity_save_additional(
            &self,
            _: &dyn LevelHost,
            event: &mut BlockEntitySaveAdditionalEvent<'_>,
        ) {
            SAVED_AT.with_borrow_mut(|seen| seen.push((event.pos, event.level)));
            let serial = self.0.fetch_add(1, Ordering::Relaxed);
            match event.tag.get(&"count".into()) {
                Some(Tag::Int(-3)) => {
                    event.tag.insert(
                        "values".into(),
                        Tag::List(vec![Tag::String(JavaString(vec![0; 32_768]))]),
                    );
                }
                Some(Tag::Int(-2)) => {
                    event.tag.insert(JavaString(vec![0; 32_768]), Tag::Int(-99));
                }
                Some(Tag::Int(-1)) => {
                    event.tag.insert("invalid".into(), Tag::End);
                }
                Some(Tag::Int(0)) => {
                    event.tag.remove(&"count".into());
                }
                Some(Tag::Int(7)) => {
                    event.tag.insert("count".into(), Tag::Int(serial));
                }
                other => panic!("unexpected payload: {other:?}"),
            }
            assert!(!event.tag.contains_key(&ATTACHMENTS_KEY.into()));
            event
                .tag
                .insert("id".into(), Tag::String("test:wrong".into()));
            event.tag.insert(SHAPES_KEY.into(), Tag::Byte(1));
            event.tag.insert(ATTACHMENTS_KEY.into(), Tag::Byte(1));
        }
    }
    struct Mod(Arc<AtomicI32>);
    #[mod_api::__subscriptions]
    impl Hooks for Mod {
        fn register(
            &self,
            registrar: &mut dyn mod_api::Registrar,
            _: &mod_api::RegistrationContext,
        ) {
            let counter = registrar.block_entity_type("counter").build();
            registrar
                .block_entity_part(&counter, Counter(self.0.clone()))
                .unwrap();
            registrar.block_entity_type("silent").build();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn structure_capture_refuses_lossy_listener_keys_and_strings() {
        let calls = Arc::new(AtomicI32::new(0));
        let mods = vec![ModEntry::new("test", Box::new(Mod(calls.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                mod_api::__private::resolve_block_entity_types(
                    ["test:counter", "test:silent"],
                    |_| Some(()),
                )
                .unwrap();
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let dirt = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:dirt")
                    .unwrap();
                for (x, selector) in [(5, -2), (6, -3)] {
                    game.world.set(x, 64, 1, dirt).unwrap();
                    game.world.block_entities.insert(
                        (x, 64, 1),
                        BlockEntity::Mod(Box::new(ModBlockEntity::new(
                            "test:counter".into(),
                            ModBlockEntityFields {
                                nbt: Compound::from([
                                    ("".into(), Tag::Int(123)),
                                    ("count".into(), Tag::Int(selector)),
                                    ("values".into(), Tag::List(vec![Tag::String("keep".into())])),
                                ]),
                                ..Default::default()
                            },
                        ))),
                    );
                }
                assert!(game.save_structure_area(
                    "test:payload",
                    [5, 64, 1],
                    [2, 1, 1],
                    true,
                    "",
                    true,
                    false
                ));
                let bytes =
                    std::fs::read(scratch.path().join("generated/test/structures/payload.nbt"))
                        .unwrap();
                let limits = Limits::templates();
                let raw = sunrise_world::compression::Compression::Gzip
                    .decode(&bytes, limits.max_bytes)
                    .unwrap();
                let (saved, _) = read_named(&raw, limits).unwrap();
                let Some(Tag::List(blocks)) = saved.tag.get("blocks") else {
                    panic!("structure blocks missing")
                };
                assert_eq!(blocks.len(), 2);
                for block in blocks {
                    let nbt = block.get("nbt").unwrap();
                    assert_eq!(nbt.get(""), Some(&Tag::Int(123)));
                    assert_eq!(
                        nbt.get("values"),
                        Some(&Tag::List(vec![Tag::String("keep".into())]))
                    );
                }
                assert_eq!(calls.load(Ordering::Relaxed), 2);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn command_and_unload_capture_the_live_payload_without_mutating_storage() {
        let count = Arc::new(AtomicI32::new(11));
        mod_platform::testing::with_mods(
            vec![ModEntry::new("test", Box::new(Mod(count.clone())))].into(),
            || {
                with_game(|game| {
                    mod_api::__private::resolve_block_entity_types(
                        ["test:counter", "test:silent"],
                        |_| Some(()),
                    )
                    .unwrap();
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    let pos = (1, 64, 1);
                    let silent = (2, 64, 1);
                    let invalid = (3, 64, 1);
                    let erased = (4, 64, 1);
                    for (pos, kind, saved) in [
                        (pos, "test:counter", 7),
                        (silent, "test:silent", 7),
                        (invalid, "test:counter", -1),
                        (erased, "test:counter", 0),
                    ] {
                        let state = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:dirt")
                            .unwrap();
                        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                        game.world.block_entities.insert(
                            pos,
                            BlockEntity::Mod(Box::new(ModBlockEntity::new(
                                kind.into(),
                                ModBlockEntityFields {
                                    nbt: Compound::from([
                                        ("count".into(), Tag::Int(saved)),
                                        (ATTACHMENTS_KEY.into(), Tag::Byte(7)),
                                    ]),
                                    ..Default::default()
                                },
                            ))),
                        );
                    }
                    let tag = game.block_entity_nbt([1, 64, 1]).unwrap();
                    assert_eq!(tag.get("count"), Some(&Tag::Int(11)));
                    assert_eq!(tag.get("id"), Some(&Tag::String("test:counter".into())));
                    assert_eq!(tag.get(ATTACHMENTS_KEY), Some(&Tag::Byte(7)));
                    assert_eq!(tag.get(SHAPES_KEY), None);

                    // Ctrl-pick must not copy the removed field in the base clone item.
                    let picked = game.picked_stack([4, 64, 1], true).unwrap();
                    assert!(
                        picked
                            .data()
                            .and_then(|d| d.other("minecraft:block_entity_data"))
                            .is_none_or(|tag| tag.get("count").is_none())
                    );
                    assert_eq!(count.load(Ordering::Relaxed), 13);

                    let detached = game.world.chunks.remove(&(0, 0)).unwrap();
                    let capture = game.with_entities([&detached]);
                    let fields = |entity: &BlockEntity| {
                        let BlockEntity::Mod(entity) = entity else {
                            panic!("expected a mod block entity")
                        };
                        entity.nbt.get(&"count".into()).cloned()
                    };
                    let snapshots = &capture[0].block_entities;
                    let stored = fields(&snapshots.iter().find(|(p, _)| *p == pos).unwrap().1);
                    assert_eq!(stored, Some(Tag::Int(7)));
                    assert_eq!(
                        fields(&snapshots.iter().find(|(p, _)| *p == silent).unwrap().1),
                        Some(Tag::Int(7))
                    );
                    assert_eq!(
                        fields(game.world.block_entities.get(&pos).unwrap()),
                        Some(Tag::Int(7))
                    );
                    assert_eq!(count.load(Ordering::Relaxed), 16);
                    // The unload capture names the entity's place after its chunk left the world.
                    let level = game.level("minecraft:overworld").unwrap();
                    SAVED_AT.with_borrow(|seen| {
                        assert!(seen.len() >= 3);
                        assert!(seen.contains(&([1, 64, 1], level)));
                        assert!(seen.iter().all(|(_, at)| *at == level));
                    });

                    // A malformed individual payload is refused at the callback seam.
                    // Other entities' valid edits still reach the actual region record.
                    let scratch = tempfile::tempdir().unwrap();
                    let save = SaveDir::new(
                        scratch.path().to_owned(),
                        LevelShape::OVERWORLD,
                        Some(Arc::new(Default::default())),
                    );
                    save.save_later(&capture, game.pack.assets.registry, game.res.biomes())
                        .unwrap();
                    save.flush().unwrap();
                    let saved = RegionFile::open(scratch.path().join("region/r.0.0.mca"))
                        .unwrap()
                        .read(ChunkPos::new(0, 0))
                        .unwrap()
                        .unwrap()
                        .nbt(Limits::saved())
                        .unwrap();
                    let Some(Tag::List(entities)) = saved.tag.get("block_entities") else {
                        panic!("expected saved block entities");
                    };
                    let at = |x| {
                        entities
                            .iter()
                            .find(|tag| tag.get("x") == Some(&Tag::Int(x)))
                            .unwrap()
                    };
                    assert!(matches!(at(1).get("count"), Some(Tag::Int(13..=15))));
                    assert_eq!(at(3).get("count"), Some(&Tag::Int(-1)));
                    assert_eq!(at(3).get("invalid"), None);
                    assert_eq!(at(4).get("count"), None);
                })
            },
        );
    }

    struct BreakObserver(Arc<Mutex<Vec<(&'static str, Option<Tag>)>>>);

    #[mod_api::__subscriptions]
    impl Hooks for BreakObserver {
        fn player_destroyed_block(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::PlayerDestroyedBlockEvent,
        ) {
            let stored = event
                .block_entity
                .as_ref()
                .and_then(|tag| tag.get(&"count".into()))
                .cloned();
            self.0.lock().unwrap().push(("removed", stored));
        }
        fn block_drops(
            &self,
            level: &dyn LevelHost,
            event: &mut mod_api::BlockDropsEvent<'_>,
        ) -> mod_api::Flow {
            let stored = event
                .block_entity
                .and_then(|entity| level.block_entity_data(entity))
                .and_then(|mut tag| tag.remove(&"count".into()));
            self.0.lock().unwrap().push(("drops", stored));
            mod_api::Flow::Continue
        }
    }

    #[test]
    #[ignore = "daily"]
    fn break_only_serializes_admitted_loot_not_removal_notices() {
        use sunrise_client::{command::args::GameMode, items::ItemStack, motion::PlayerMotion};
        use sunrise_core::geometry::Vec3;

        for with_observers in [false, true] {
            let count = Arc::new(AtomicI32::new(11));
            let seen = Arc::new(Mutex::new(Vec::new()));
            let mut mods = vec![ModEntry::new("test", Box::new(Mod(count.clone())))];
            if with_observers {
                mods.push(ModEntry::new(
                    "test_observer",
                    Box::new(BreakObserver(seen.clone())),
                ));
            }
            mod_platform::testing::with_mods(mods.into(), || {
                with_game(|game| {
                    mod_api::__private::resolve_block_entity_types(
                        ["test:counter", "test:silent"],
                        |_| Some(()),
                    )
                    .unwrap();
                    game.options.dedicated = Some(Vec::new());
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.block_ticks
                        .add_empty_chunk(ChunkPos::new(0, 0))
                        .unwrap();
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 64.0, 4.5)).unwrap());
                    game.lan_acting_uuid = Some(123);
                    game.lan_acting_entity = Some(20);
                    game.lan_acting_permission = Some(0);
                    let place = |game: &mut crate::game::Game<'_>, name| {
                        let pos = (7, 64, 4);
                        let state = game.pack.assets.registry.default_state(name).unwrap();
                        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
                        game.world.block_entities.insert(
                            pos,
                            BlockEntity::Mod(Box::new(ModBlockEntity::new(
                                "test:counter".into(),
                                ModBlockEntityFields {
                                    nbt: Compound::from([("count".into(), Tag::Int(7))]),
                                    ..Default::default()
                                },
                            ))),
                        );
                    };
                    for (mode, name, tool, serials) in [
                        (GameMode::Survival, "minecraft:dirt", None, 12),
                        (GameMode::Survival, "minecraft:diamond_ore", None, 12),
                        (
                            GameMode::Creative,
                            "minecraft:dirt",
                            Some(ItemStack::new("minecraft:stick", 1)),
                            12,
                        ),
                    ] {
                        game.set_mode(mode);
                        game.inventory[game.selected] = tool;
                        place(game, name);
                        assert!(game.break_block_as_player([7, 64, 4]).unwrap());
                        assert_eq!(count.load(Ordering::Relaxed), serials, "{mode:?}, {name}");
                    }
                    let seen = seen.lock().unwrap();
                    if with_observers {
                        assert_eq!(
                            *seen,
                            [
                                ("removed", Some(Tag::Int(7))),
                                ("drops", Some(Tag::Int(11))),
                                ("removed", Some(Tag::Int(7))),
                                ("removed", Some(Tag::Int(7))),
                            ]
                        );
                    } else {
                        assert!(seen.is_empty());
                    }
                })
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn final_drops_read_the_same_outgoing_serializer_across_destruction_routes() {
        use crate::game::explosion::BlockInteraction;
        use mod_api::{BlockDestructionCause as Cause, LevelDestructionCause as LevelCause};
        use sunrise_client::{command::args::GameMode, mobs::MobEvent, motion::PlayerMotion};
        use sunrise_core::geometry::Vec3;

        type Seen = (Cause, Option<mod_api::BlockEntityRef>);
        struct ObserveRoll(Arc<Mutex<Vec<Seen>>>);
        #[mod_api::__subscriptions]
        impl Hooks for ObserveRoll {
            fn block_drops(
                &self,
                level: &dyn LevelHost,
                event: &mut mod_api::BlockDropsEvent<'_>,
            ) -> mod_api::Flow {
                self.0
                    .lock()
                    .unwrap()
                    .push((event.cause(), event.block_entity));
                let [drop] = event.drops.as_slice() else {
                    panic!("expected one rolled drop")
                };
                let copied = drop
                    .stack
                    .components()
                    .and_then(|patch| patch.as_nbt().get(&"minecraft:custom_data".into()));
                match event.block_entity {
                    Some(entity) => {
                        let mut payload =
                            level.block_entity_data(entity).expect("outgoing payload");
                        let serial = payload.remove(&"count".into()).expect("serializer output");
                        assert_eq!(copied.and_then(|tag| tag.get("serial")), Some(&serial));
                        assert_eq!(
                            copied.and_then(|tag| tag.get("name")),
                            payload.get(&"CustomName".into())
                        );
                    }
                    None => assert!(copied.is_none()),
                }
                mod_api::Flow::Continue
            }
        }
        let serial = Arc::new(AtomicI32::new(12));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![
            ModEntry::new("test", Box::new(Mod(serial.clone()))),
            ModEntry::new("observer", Box::new(ObserveRoll(seen.clone()))),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                mod_api::__private::resolve_block_entity_types(
                    ["test:counter", "test:silent"],
                    |_| Some(()),
                )
                .unwrap();
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 64.0, 4.5)).unwrap());
                game.lan_acting_uuid = Some(123);
                game.lan_acting_entity = Some(20);
                game.lan_acting_permission = Some(0);
                game.set_mode(GameMode::Survival);
                let mut items = game.items().clone();
                items.set_loot_tables(Arc::new(["dirt", "poppy"].into_iter().map(|block| {
                    (format!("minecraft:blocks/{block}"), serde_json::json!({
                        "pools": [{"rolls": 1, "entries": [{"type": "minecraft:item", "name": "minecraft:emerald",
                            "modifier": {"type": "minecraft:copy_custom_data", "source": "block_entity", "ops": [
                                {"source": "count", "target": "serial", "op": "replace"},
                                {"source": "CustomName", "target": "name", "op": "replace"}
                            ]}
                        }]}]
                    }))
                }).collect()));
                game.server_items = Some(Arc::new(items));
                let at = [7, 64, 4];
                let reg = game.pack.assets.registry;
                let dirt = reg.default_state("minecraft:dirt").unwrap();
                let poppy = reg.default_state("minecraft:poppy").unwrap();
                let power = reg.default_state("minecraft:redstone_block").unwrap();
                let piston = game
                    .with_props(
                        reg.default_state("minecraft:piston").unwrap(),
                        &[("facing", "east")],
                    )
                    .unwrap();
                let direct = game.loot_entity_ref(Some(91));
                let owner = game.loot_entity_ref(Some(20));
                assert!(direct.is_some() && owner.is_some());
                for (kind, expected) in [
                    ("player", Cause::PlayerBreak),
                    ("mob", Cause::MobBreak),
                    ("piston", Cause::Level(LevelCause::Piston)),
                    ("command", Cause::Level(LevelCause::Command)),
                    ("in-flight", Cause::Level(LevelCause::Command)),
                    (
                        "blast",
                        Cause::Explosion {
                            radius: 4.0,
                            direct_source: direct,
                            causing_entity: owner,
                        },
                    ),
                    ("entity-free", Cause::PlayerBreak),
                ] {
                    game.dropped.clear();
                    let before = serial.load(Ordering::Relaxed);
                    let state = if kind == "piston" { poppy } else { dirt };
                    game.world.set(at[0], at[1], at[2], state).unwrap();
                    if kind != "entity-free" {
                        game.world.block_entities.insert(
                            (at[0], at[1], at[2]),
                            BlockEntity::Mod(Box::new(ModBlockEntity::new(
                                "test:counter".into(),
                                ModBlockEntityFields {
                                    nbt: Compound::from([("count".into(), Tag::Int(7))]),
                                    ..Default::default()
                                },
                            ))),
                        );
                    }
                    match kind {
                        "player" | "entity-free" => {
                            assert!(game.break_block_as_player(at).unwrap())
                        }
                        "mob" => game
                            .handle_mob_events(vec![MobEvent::BreakBlock {
                                pos: at,
                                drop: true,
                                by: None,
                                breaker: None,
                            }])
                            .unwrap(),
                        "piston" => {
                            game.world.set(7, 63, 4, dirt).unwrap();
                            game.world.set(6, 64, 4, piston).unwrap();
                            game.world.set(5, 64, 4, power).unwrap();
                            game.with_level(|b, level, ticks| {
                                assert!(b.client_block_event(
                                    level,
                                    ticks,
                                    sunrise_core::position::BlockPos::new(6, 64, 4),
                                    0,
                                    sunrise_core::geometry::Direction::East as i32
                                ));
                            });
                        }
                        "command" => game.destroy_block_dropping(at).unwrap(),
                        "in-flight" => {
                            use sunrise_core::{geometry::Direction, position::BlockPos};
                            use sunrise_world::level::MovingPiston;
                            let from = BlockPos::new(at[0], at[1], at[2]);
                            let to = BlockPos::new(8, 64, 4);
                            let name = Tag::compound([("text", Tag::String("Travelling".into()))]);
                            game.world
                                .block_names
                                .insert((from.x, from.y, from.z), name);
                            game.with_level(|_, level, ticks| {
                                level.carry_mod_block_entity(from, to);
                                ticks.moving_pistons.push((
                                    to,
                                    MovingPiston {
                                        moved_state: dirt,
                                        direction: Direction::East,
                                        extending: true,
                                        source: false,
                                        progress: 0.5,
                                        progress_o: 0.0,
                                        last_ticked: 0,
                                        death_ticks: 0,
                                        id: 0,
                                    },
                                ));
                            });
                            let moving = reg.default_state("minecraft:moving_piston").unwrap();
                            game.world
                                .set(from.x, from.y, from.z, game.world.air)
                                .unwrap();
                            game.world.set(to.x, to.y, to.z, moving).unwrap();
                            let key = (to.x, to.y, to.z);
                            assert!(game.world.block_entities.carried(&key).is_some());
                            assert!(game.world.block_entities.get(&key).is_none());
                            game.destroy_block_dropping([to.x, to.y, to.z]).unwrap();
                            assert!(game.world.block_entities.carried(&key).is_none());
                        }
                        "blast" => {
                            let start = mod_api::ExplosionStart::from(
                                mod_api::__private::built::ExplosionStart {
                                    dimension: "minecraft:overworld",
                                    center: [0.0; 3],
                                    radius: 4.0,
                                    fire: false,
                                    interaction: mod_api::BlockInteraction::Destroy,
                                    damage_type: "minecraft:explosion",
                                    source: Some(91),
                                    owner: Some(20),
                                },
                            );
                            assert_eq!(
                                game.explosion_block_step(
                                    &mut [at],
                                    4.0,
                                    BlockInteraction::Destroy,
                                    false,
                                    &start
                                )
                                .unwrap(),
                                1
                            );
                        }
                        _ => unreachable!(),
                    }
                    let heard = std::mem::take(&mut *seen.lock().unwrap());
                    let [(cause, snapshot)] = heard.as_slice() else {
                        panic!("expected one {kind} event, got {heard:?}")
                    };
                    assert_eq!(*cause, expected, "{kind}");
                    assert_eq!(game.dropped.len(), 1, "{kind}");
                    let item = &game.dropped[0].stack;
                    assert_eq!(
                        (&*item.item, item.count),
                        ("minecraft:emerald", 1),
                        "{kind}"
                    );
                    if kind == "entity-free" {
                        assert!(snapshot.is_none());
                        assert!(item.custom_data().is_none());
                        assert_eq!(serial.load(Ordering::Relaxed), before);
                    } else {
                        let copied = item.custom_data().unwrap();
                        assert_eq!(
                            copied.get(&"serial".into()),
                            Some(&Tag::Int(before)),
                            "{kind}"
                        );
                        if kind == "in-flight" {
                            assert_eq!(
                                copied.get(&"name".into()),
                                Some(&Tag::compound([("text", Tag::String("Travelling".into()))]))
                            );
                        }
                        assert_eq!(
                            serial.load(Ordering::Relaxed),
                            before + 1,
                            "{kind}: one serialization"
                        );
                        game.with_level(|_, level, _| {
                            assert!(level.block_entity_data(snapshot.unwrap()).is_none())
                        });
                    }
                }
            })
        });
    }
}
