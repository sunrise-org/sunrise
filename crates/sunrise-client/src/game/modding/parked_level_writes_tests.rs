#![cfg(test)]

use super::{Dim, Game, api_values::TryIntoApi, native_entity_refs::tests::with_game};
use mod_api::{
    BlockChangedEvent, BlockEntityError, BlockState, BlockStateError, BlockStateRecord,
    BlockUpdateFlags, Compound, Hooks, HostTransaction as _, LevelDimension, LevelHost, LevelRef,
    LevelWriteError, ServerHost,
};
use std::sync::{Arc, Mutex};
use sunrise_client::chunk::Chunk;
use sunrise_core::nbt::Tag;
use sunrise_world::chunk::BlockState as SavedState;

const POS: [i32; 3] = [2, 64, 2];
const KEY: (i32, i32, i32) = (2, 64, 2);
const OVERWORLD: &str = "minecraft:overworld";
const NETHER: &str = "minecraft:the_nether";

type Heard = (LevelDimension, BlockState, BlockState, BlockUpdateFlags);
#[derive(Clone, Default)]
struct Changes(Arc<Mutex<Vec<Heard>>>);

#[mod_api::__subscriptions]
impl Hooks for Changes {
    fn block_changed(&self, host: &dyn LevelHost, event: &mut BlockChangedEvent) {
        if [event.pos.x, event.pos.y, event.pos.z] == POS {
            assert_eq!(host.block_state(POS), Some(event.new));
            self.0
                .lock()
                .unwrap()
                .push((event.dimension, event.old, event.new, event.flags));
        }
    }
}

fn chest_states(game: &Game<'_>) -> [BlockState; 3] {
    let registry = game.pack.assets.registry;
    ["north", "east", "south"].map(|facing| {
        registry
            .resolve(&SavedState {
                name: "minecraft:chest".into(),
                properties: [
                    ("facing", facing),
                    ("type", "single"),
                    ("waterlogged", "false"),
                ]
                .map(|(key, value)| (key.into(), value.into()))
                .into(),
            })
            .unwrap()
            .try_into_api()
            .unwrap()
    })
}

fn with_two_levels(
    test: impl FnOnce(&mut Game<'_>, LevelRef, LevelRef, [BlockState; 3], &Mutex<Vec<Heard>>),
) {
    let changes = Changes::default();
    let mods = vec![mod_api::ModEntry::new(
        "parked_level_test",
        Box::new(changes.clone()),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            game.world_root = game.options.game_dir.clone().unwrap();
            let states = chest_states(game);
            for dim in [Dim::Nether, Dim::Overworld] {
                game.enter_level(dim).unwrap();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let level = game.level(dim.id()).unwrap();
                assert_eq!(
                    level.set_block(game, POS, states[0], BlockUpdateFlags::KNOWN_SHAPE),
                    Ok(true)
                );
                game.world
                    .block_entities
                    .set_attachments(KEY, Some(Default::default()));
                game.world.block_log = Some(Vec::new());
                game.world.modified.clear();
            }
            let current = game.level(OVERWORLD).unwrap();
            let parked = game.level(NETHER).unwrap();
            changes.0.lock().unwrap().clear();
            test(game, current, parked, states, &changes.0);
            assert_eq!(game.dim, Dim::Overworld);
        });
    });
}

fn payload(count: u8) -> Compound {
    let snbt = format!(
        r#"{{id:"minecraft:furnace",x:100,y:3,z:99,Items:[{{Slot:0b,id:"minecraft:stick",count:{count}}}]}}"#
    );
    let Tag::Compound(data) = sunrise_client::command::snbt::parse_fully(&snbt).unwrap() else {
        unreachable!()
    };
    data
}

fn sticks(game: &Game<'_>, level: LevelRef) -> Option<u32> {
    let world = game.native_level_world(level).unwrap();
    let entity = world.block_entities.get(&KEY).unwrap();
    entity.slots()[0].as_ref().map(|stack| {
        assert_eq!(stack.item.as_ref(), "minecraft:stick");
        stack.count
    })
}

#[test]
#[ignore = "daily"]
fn parked_property_and_payload_writes_reach_only_the_addressed_entity() {
    with_two_levels(|game, current, parked, [north, east, _], heard| {
        let entity = game.block_entity(parked, POS).unwrap();
        let flags = BlockUpdateFlags::KNOWN_SHAPE | BlockUpdateFlags::CLIENTS;
        assert_eq!(parked.set_block(game, POS, east, flags), Ok(true));
        assert_eq!(
            *heard.lock().unwrap(),
            [(LevelDimension::Nether, north, east, flags)]
        );
        assert_eq!(current.block_state(game, POS), Some(north));
        assert_eq!(parked.block_state(game, POS), Some(east));
        assert_eq!(game.block_entity(parked, POS), Some(entity));
        assert_eq!(
            game.native_level_world(parked)
                .unwrap()
                .block_log
                .as_deref(),
            Some(&[KEY][..])
        );
        assert_eq!(entity.merge_data(game, payload(3)), Ok(true));
        assert_eq!(entity.position(game), Some(POS));
        assert_eq!(game.block_entity(parked, POS), Some(entity));
        assert_eq!(sticks(game, parked), Some(3));
        assert_eq!(sticks(game, current), None);
        let world = game.native_level_world(parked).unwrap();
        assert!(world.block_entities.attachments(KEY).is_some());
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn root_commit_uses_final_flags_and_keeps_same_position_levels_separate() {
    with_two_levels(|game, current, parked, [north, east, south], heard| {
        let entity = game.block_entity(parked, POS).unwrap();
        let first = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS;
        let last = BlockUpdateFlags::KNOWN_SHAPE;
        (game as &mut dyn ServerHost)
            .transaction(|host, _| {
                assert_eq!(parked.set_block(host, POS, east, first), Ok(true));
                assert_eq!(current.set_block(host, POS, east, last), Ok(true));
                host.transaction(|host, _| {
                    assert_eq!(parked.set_block(host, POS, south, last), Ok(true));
                    assert_eq!(entity.replace_data(host, payload(7)), Ok(true));
                    Ok::<_, ()>(())
                })?;
                assert_eq!(parked.block_state(host, POS), Some(south));
                assert_eq!(current.block_state(host, POS), Some(east));
                assert!(entity.data(host).unwrap().contains_key(&"Items".into()));
                assert!(heard.lock().unwrap().is_empty());
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(
            *heard.lock().unwrap(),
            [
                (LevelDimension::Nether, north, south, last),
                (LevelDimension::Overworld, north, east, last)
            ]
        );
        assert_eq!(game.block_entity(parked, POS), Some(entity));
        assert_eq!(sticks(game, parked), Some(7));
        assert_eq!(sticks(game, current), None);
        // Only the saved-data load asks to send a client update (the last block flags don't).
        assert_eq!(
            game.native_level_world(parked)
                .unwrap()
                .block_log
                .as_deref(),
            Some(&[KEY][..])
        );
        assert_eq!(game.world.block_log.as_deref(), Some(&[][..]));
    });
}

#[test]
#[ignore = "daily"]
fn outer_abort_restores_state_and_payload_without_notices_or_dirtying_chunks() {
    with_two_levels(|game, current, parked, [north, east, south], heard| {
        let entity = game.block_entity(parked, POS).unwrap();
        assert_eq!(entity.merge_data(game, payload(2)), Ok(true));
        game.with_resident_block_level(Dim::Nether, |game| {
            game.world.block_log.as_mut().unwrap().clear();
            game.world.modified.clear();
        })
        .unwrap();
        let flags = BlockUpdateFlags::CLIENTS | BlockUpdateFlags::KNOWN_SHAPE;
        let result = (game as &mut dyn ServerHost).transaction(|host, _| {
            assert_eq!(parked.set_block(host, POS, east, flags), Ok(true));
            host.transaction(|host, _| {
                assert_eq!(parked.set_block(host, POS, south, flags), Ok(true));
                assert_eq!(entity.replace_data(host, payload(9)), Ok(true));
                Ok::<_, ()>(())
            })?;
            Err::<(), ()>(())
        });
        assert_eq!(result, Err(()));
        assert!(heard.lock().unwrap().is_empty());
        assert_eq!(parked.block_state(game, POS), Some(north));
        assert_eq!(current.block_state(game, POS), Some(north));
        assert_eq!(game.block_entity(parked, POS), Some(entity));
        assert_eq!(sticks(game, parked), Some(2));
        let world = game.native_level_world(parked).unwrap();
        assert_eq!(world.block_log.as_deref(), Some(&[][..]));
        assert!(!world.modified.contains(&(0, 0)));
    });
}

#[test]
#[ignore = "daily"]
fn adjacent_replay_does_not_break_a_tentative_wall_torch() {
    with_two_levels(|game, _, parked, _, _| {
        let stone = (&*game as &dyn ServerHost)
            .block_state_id("minecraft:stone")
            .unwrap();
        let air = (&*game as &dyn ServerHost)
            .block_state_id("minecraft:air")
            .unwrap();
        let torch = (&*game as &dyn ServerHost)
            .block_state_id("minecraft:wall_torch[facing=east]")
            .unwrap();
        let next = [3, 64, 2];
        assert_eq!(
            parked.set_block(game, POS, stone, BlockUpdateFlags::KNOWN_SHAPE),
            Ok(true)
        );
        (game as &mut dyn ServerHost)
            .transaction(|host, _| {
                assert_eq!(
                    parked.set_block(host, POS, air, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                assert_eq!(
                    parked.set_block(host, next, torch, BlockUpdateFlags::KNOWN_SHAPE),
                    Ok(true)
                );
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(parked.block_state(game, POS), Some(air));
        assert_eq!(parked.block_state(game, next), Some(torch));
        game.with_resident_block_level(Dim::Nether, |game| {
            assert!(
                game.dropped.is_empty(),
                "the torch wasn't placed when its support was replayed"
            );
        })
        .unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn tentative_holder_and_payload_follow_the_outer_transaction() {
    with_two_levels(|game, current, parked, [north, _, _], heard| {
        let original = game.block_entity(parked, POS).unwrap();
        let barrel = (&*game as &dyn ServerHost)
            .block_state_id("minecraft:barrel")
            .unwrap();
        let flags = BlockUpdateFlags::KNOWN_SHAPE;
        let mut aborted_holder = None;
        let aborted = (game as &mut dyn ServerHost).transaction(|host, _| {
            host.transaction(|host, _| {
                assert_eq!(parked.set_block(host, POS, barrel, flags), Ok(true));
                let staged = host.block_entity(parked, POS).unwrap();
                assert_ne!(staged, original);
                aborted_holder = Some(staged);
                assert_eq!(staged.merge_data(host, payload(4)), Ok(true));
                assert_eq!(
                    staged.data(host).unwrap().get(&"Items".into()),
                    payload(4).get(&"Items".into())
                );
                Ok::<_, ()>(())
            })?;
            assert_eq!(host.block_entity(parked, POS), aborted_holder);
            assert!(heard.lock().unwrap().is_empty());
            Err::<(), ()>(())
        });
        assert_eq!(aborted, Err(()));
        assert!(aborted_holder.unwrap().data(game).is_none());
        assert_eq!(game.block_entity(parked, POS), Some(original));
        assert_eq!(parked.block_state(game, POS), Some(north));
        assert_eq!(sticks(game, parked), None);
        assert!(heard.lock().unwrap().is_empty());
        assert!(
            game.native_level_world(parked)
                .unwrap()
                .block_log
                .as_ref()
                .unwrap()
                .is_empty()
        );

        let staged = (game as &mut dyn ServerHost)
            .transaction(|host, _| {
                assert_eq!(parked.set_block(host, POS, barrel, flags), Ok(true));
                let staged = host.block_entity(parked, POS).unwrap();
                assert_ne!(Some(staged), aborted_holder);
                assert_eq!(staged.replace_data(host, payload(6)), Ok(true));
                Ok::<_, ()>(staged)
            })
            .unwrap();
        assert_eq!(game.block_entity(parked, POS), Some(staged));
        assert_eq!(sticks(game, parked), Some(6));
        assert_eq!(sticks(game, current), None);
        assert_eq!(
            *heard.lock().unwrap(),
            [(LevelDimension::Nether, north, barrel, flags)]
        );
    });
}

#[test]
#[ignore = "daily"]
fn committed_saved_jukebox_record_without_a_song_clock_stays_idle() {
    use sunrise_client::containers::BlockEntity;
    with_two_levels(|game, _, parked, _, _| {
        let jukebox = (&*game as &dyn ServerHost)
            .block_state_id("minecraft:jukebox")
            .unwrap();
        let flags = BlockUpdateFlags::KNOWN_SHAPE;
        let Tag::Compound(record) = sunrise_client::command::snbt::parse_fully(
            r#"{RecordItem:{id:"minecraft:music_disc_cat",count:1}}"#,
        )
        .unwrap() else {
            unreachable!()
        };
        let other = [3, 64, 2];
        for at in [POS, other] {
            assert_eq!(parked.set_block(game, at, jukebox, flags), Ok(true));
        }
        let synchronous = game.block_entity(parked, other).unwrap();
        assert_eq!(synchronous.replace_data(game, record.clone()), Ok(true));
        let transactional = game.block_entity(parked, POS).unwrap();
        (game as &mut dyn ServerHost)
            .transaction(|host, _| {
                assert_eq!(transactional.replace_data(host, record), Ok(true));
                Ok::<_, ()>(())
            })
            .unwrap();
        let world = game.native_level_world(parked).unwrap();
        for key in [KEY, (3, 64, 2)] {
            let Some(BlockEntity::Jukebox(entity)) = world.block_entities.get(&key) else {
                panic!("expected a jukebox at {key:?}");
            };
            assert_eq!(
                entity.record().map(|record| record.item.as_ref()),
                Some("minecraft:music_disc_cat")
            );
            assert!(!entity.playing);
            assert!(!entity.song_clock_present);
        }
    });
}

#[test]
#[ignore = "daily"]
fn parked_removal_sound_and_queued_song_reach_only_target_guests() {
    use super::native_sound_refs::tests::{guest, packets};
    use sunrise_client::lan::{NetConfig, hub::HubConfig};
    use sunrise_protocol::packets::play::Clientbound;
    with_two_levels(|game, _, parked, _, _| {
        let beacon = (&*game as &dyn ServerHost)
            .block_state_id("minecraft:beacon")
            .unwrap();
        let jukebox = (&*game as &dyn ServerHost)
            .block_state_id("minecraft:jukebox")
            .unwrap();
        let air = (&*game as &dyn ServerHost)
            .block_state_id("minecraft:air")
            .unwrap();
        let flags = BlockUpdateFlags::KNOWN_SHAPE;
        let song_at = [3, 64, 2];
        assert_eq!(parked.set_block(game, POS, beacon, flags), Ok(true));
        assert_eq!(parked.set_block(game, song_at, jukebox, flags), Ok(true));
        let Tag::Compound(record) = sunrise_client::command::snbt::parse_fully(
            r#"{RecordItem:{id:"minecraft:music_disc_cat",count:1}}"#,
        )
        .unwrap() else {
            unreachable!()
        };
        let entity = game.block_entity(parked, song_at).unwrap();
        assert_eq!(entity.replace_data(game, record), Ok(true));

        let net = NetConfig {
            listen: false,
            announce: false,
            lod_distance: 0,
            ..NetConfig::lan(0, String::new(), 3)
        };
        let config = HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: OVERWORLD.into(),
            allow_flight: true,
            seed: 0,
            max_players: 3,
            integrated: true,
            levels: game.level_shapes,
            level_seeds: game.levels.runtime.seeds(),
        };
        let settings = super::lan::LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        };
        game.lan_host(settings, net, config).unwrap();
        let (mut overworld, rx_overworld) = guest(21, 21, [2.5, 64.5, 2.5]);
        overworld.dimension = OVERWORLD.into();
        let (mut nether, rx_nether) = guest(22, 22, [2.5, 64.5, 2.5]);
        nether.dimension = NETHER.into();
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .extend([overworld, nether]);

        assert_eq!(parked.set_block(game, POS, air, flags), Ok(true));
        game.with_resident_block_level(Dim::Nether, |game| game.jukebox_inserted(song_at))
            .unwrap();
        game.lan_flush_effects();
        let heard = |packet: &Clientbound| {
            matches!(
                packet,
                Clientbound::Sound(_)
                    | Clientbound::LevelEvent(sunrise_protocol::packets::play::LevelEvent {
                        kind: 1010,
                        ..
                    })
            )
        };
        assert!(!packets(&rx_overworld).iter().any(heard));
        assert!(!packets(&rx_nether).iter().any(heard));
        game.with_resident_block_level(Dim::Nether, |game| game.lan_flush_effects())
            .unwrap();
        assert!(!packets(&rx_overworld).iter().any(heard));
        let packets = packets(&rx_nether);
        let sound = game.lan.as_ref().unwrap().hub.sound_packet(
            "minecraft:block.beacon.deactivate",
            [2.5, 64.5, 2.5],
            1.0,
            1.0,
            0,
            0,
        );
        let Clientbound::Sound(expected) = sound else {
            panic!("expected the beacon sound");
        };
        assert_eq!(
            packets
                .iter()
                .filter_map(|packet| match packet {
                    Clientbound::Sound(p) => Some(format!("{:?}", p.sound)),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [format!("{:?}", expected.sound)]
        );
        assert_eq!(
            packets
                .iter()
                .filter(|packet| matches!(packet,
            Clientbound::LevelEvent(p) if p.kind == 1010 && p.pos == song_at))
                .count(),
            1
        );
    });
}

#[test]
#[ignore = "daily"]
fn stale_or_unloaded_targets_never_write_or_load() {
    with_two_levels(|game, current, parked, [north, east, _], heard| {
        let entity = game.block_entity(parked, POS).unwrap();
        let stale =
            LevelRef::with_generation(parked.epoch(), parked.dimension(), parked.generation() + 1);
        let flags = BlockUpdateFlags::CLIENTS;
        assert_eq!(
            stale.set_block(game, POS, east, flags),
            Err(LevelWriteError::Stale)
        );
        assert_eq!(
            parked.set_block(
                game,
                POS,
                BlockStateRecord::new(east.block(), u16::MAX).into(),
                flags
            ),
            Err(LevelWriteError::InvalidState(BlockStateError::UnknownState))
        );
        game.with_resident_block_level(Dim::Nether, |game| game.world.chunks.remove(&(0, 0)))
            .unwrap();
        assert_eq!(parked.set_block(game, POS, east, flags), Ok(false));
        assert_eq!(
            entity.merge_data(game, payload(4)),
            Err(BlockEntityError::Stale)
        );
        assert!(game.native_level_world(parked).unwrap().chunks.is_empty());
        assert!(game.level("minecraft:the_end").is_none());
        assert_eq!(current.block_state(game, POS), Some(north));
        assert!(heard.lock().unwrap().is_empty());
    });
}
