//! Daily behavioral regressions using actual Worlds, guest packets and menus.
use super::*;
use crate::game::{Dim, native_guest_turn_tests::with_guests};
use mod_api::{Hooks, ModEntry, RuntimeGenerator, RuntimeLevel};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};
use sunrise_client::{
    chunk::Chunk,
    containers::{BlockEntity, Furnace, FurnaceKind},
    items::ItemStack,
};
use sunrise_protocol::packets::{
    configuration::CustomPayload,
    play::{self, Clientbound as C, Serverbound as S},
};

struct Fixture(Arc<AtomicU8>);
#[mod_api::__subscriptions]
impl Hooks for Fixture {
    fn break_block(
        &self,
        host: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::BreakBlockEvent,
    ) -> mod_api::Flow {
        match self.0.load(Ordering::Relaxed) {
            1 => {
                host.player(12)
                    .unwrap()
                    .set_inventory_slot(host, 0, None)
                    .unwrap();
            }
            2 => {
                host.runtime_levels().unload("input_probe:source").unwrap();
            }
            _ => {}
        }
        mod_api::Flow::Continue
    }
    fn live_world_admission(
        &self,
        host: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::LiveWorldAdmissionEvent,
    ) -> mod_api::Flow {
        match self.0.load(Ordering::Relaxed) {
            5 => {
                host.player(12)
                    .unwrap()
                    .set_inventory_slot(host, 0, None)
                    .unwrap();
            }
            6 => {
                host.runtime_levels().unload("input_probe:source").unwrap();
            }
            7 => {
                let source = host.level("input_probe:source").unwrap();
                source
                    .live_world(host)
                    .unwrap()
                    .unwrap()
                    .retire(host)
                    .unwrap();
            }
            8 => {
                assert!(host.player(12).unwrap().set_selected_slot(host, 1));
            }
            _ => {}
        }
        mod_api::Flow::Continue
    }
    fn container_opened(
        &self,
        host: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::ContainerOpenedEvent<'_>,
    ) -> mod_api::Flow {
        match self.0.load(Ordering::Relaxed) {
            10 => {
                host.player(12)
                    .unwrap()
                    .set_inventory_slot(host, 0, None)
                    .unwrap();
            }
            11 => {
                host.runtime_levels().unload("input_probe:source").unwrap();
            }
            12 => {
                let source = host.level("input_probe:source").unwrap();
                source
                    .live_world(host)
                    .unwrap()
                    .unwrap()
                    .retire(host)
                    .unwrap();
            }
            13 => {
                assert!(host.player(12).unwrap().set_selected_slot(host, 1));
            }
            _ => {}
        }
        mod_api::Flow::Continue
    }
}
fn owner() -> mod_api::content::crash::ModGuard {
    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id: "input_probe",
        hook: "level_tick_post",
        part: None,
    })
}

fn setup(game: &mut Game<'_>) -> (LiveWorldRef, impl Fn() -> Vec<Vec<u8>> + use<>) {
    use sunrise_client::{
        lan::PlayerSender,
        raw_payloads::{Local, Side, Transport},
    };
    // Both projection metadata and action channels are genuinely negotiated; neither brand nor
    // a directly inserted mod authority is used by the server packet action below.
    let local = Local::capture(Side::Server).unwrap();
    let transport = Transport::new(local.clone(), Side::Server);
    transport.accept(local.manifest.entries()).unwrap();
    transport.finish_configuration().unwrap();
    transport.bind_player(game.player(12).unwrap());
    let (mut sender, output) = PlayerSender::payload_executor_test_sender(transport);
    sender.viewed_levels = sunrise_protocol::viewed_levels::Answer {
        revision: sunrise_protocol::viewed_levels::REVISION_4,
        max_levels: 4,
    };
    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
        guest.sender = sender;
        guest.position = [10.5, 64.0, 13.0];
        guest.loaded = true;
        guest.game_mode = 1;
        guest.inventory.fill(None);
    });
    // The second synthetic guest is not part of this fixture's negotiated view.
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(1, |guest| guest.dimension = "minecraft:the_nether".into());
    let _owner = owner();
    let source = game
        .runtime_level_open(
            "input_probe:source",
            RuntimeLevel::new("minecraft:overworld", RuntimeGenerator::Void, 1022),
        )
        .unwrap();
    game.apply_runtime_levels();
    game.enter_level(Dim::from_id(source.dimension_id()))
        .unwrap();
    for x in [0, 1] {
        game.world
            .chunks
            .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
        game.streaming.forced.insert((x, 0));
    }
    for (cell, name) in [
        ([0, 64, 0], "minecraft:chest"),
        ([1, 64, 0], "minecraft:furnace"),
        ([17, 64, 0], "minecraft:stone"),
    ] {
        let state = game.pack.assets.registry.default_state(name).unwrap();
        game.world.set(cell[0], cell[1], cell[2], state).unwrap();
    }
    let mut slots = vec![None; 27];
    slots[0] = Some(ItemStack::new("minecraft:stone", 7));
    game.world
        .block_entities
        .insert((0, 64, 0), BlockEntity::Chest(slots));
    game.world.block_entities.insert(
        (1, 64, 0),
        BlockEntity::Furnace(Furnace {
            kind: FurnaceKind::Furnace,
            items: [
                Some(ItemStack::new("minecraft:raw_iron", 1)),
                Some(ItemStack::new("minecraft:coal", 1)),
                None,
            ],
            lit_remaining: 5,
            lit_total: 200,
            cooking_timer: 40,
            cooking_total: 200,
            speed: 1.0,
            recipes_used: HashMap::new(),
        }),
    );
    game.enter_level(Dim::Overworld).unwrap();
    for x in -1..=2 {
        for z in -1..=1 {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
        }
    }
    // Sentinel at the same LOCAL coordinate: a wrong-world menu read/write corrupts this.
    let chest = game
        .pack
        .assets
        .registry
        .default_state("minecraft:chest")
        .unwrap();
    game.world.set(0, 64, 0, chest).unwrap();
    let mut parent = vec![None; 27];
    parent[0] = Some(ItemStack::new("minecraft:gold_ingot", 29));
    game.world
        .block_entities
        .insert((0, 64, 0), BlockEntity::Chest(parent));
    let destination = game.level("minecraft:overworld").unwrap();
    let handle = source
        .project(
            game,
            destination,
            &[[0, 64, 0], [1, 64, 0], [17, 64, 0]],
            LiveWorldPose::at([10.0, 0.0, 10.0]),
            LiveWorldLimits::default(),
        )
        .unwrap();
    (handle, output)
}

fn request(
    game: &Game<'_>,
    handle: LiveWorldRef,
    target: [f64; 3],
    action: Action,
    sequence: i32,
) -> Request {
    let eye = game.lan.as_ref().unwrap().hub.guest(12).unwrap().eye();
    let _owner = owner();
    let hit = game.live_world_clip(handle, eye, target).unwrap().unwrap();
    Request {
        action,
        generation: handle.generation(),
        membership: game.live_worlds.borrow().entries[&handle.generation()].membership,
        pose_revision: hit.pose_revision,
        source_revision: hit.cell_revision,
        cell: hit.cell,
        face: crate::game::remote::face_direction(hit.face.normal()),
        point: hit.point,
        local_point: hit.local_point,
        hand: 0,
        sequence,
    }
}
fn send(game: &mut Game<'_>, request: Request) {
    // The owner guard has ended: this is the same authenticated Game packet entry as a guest.
    game.lan_guest_packet(
        12,
        S::CustomPayload(CustomPayload {
            channel: wire::CHANNEL.into(),
            data: wire::encode(&request).unwrap(),
        }),
    );
}
fn packets(output: &dyn Fn() -> Vec<Vec<u8>>) -> Vec<C> {
    output()
        .iter()
        .map(|body| {
            let mut reader = sunrise_protocol::buf::Reader::new(body);
            let id = reader.varint().unwrap();
            C::decode(id, reader.rest()).unwrap()
        })
        .collect()
}
fn source_state(
    game: &Game<'_>,
    handle: LiveWorldRef,
    cell: [i32; 3],
) -> sunrise_world::registry::BlockStateId {
    game.native_level_world(handle.level())
        .unwrap()
        .block(cell[0], cell[1], cell[2])
}

#[test]
#[ignore = "daily"]
fn authenticated_projection_packets_keep_real_source_menus_and_refuse_stale_actions() {
    const NAME: &str = "game::native_live_worlds::picking::tests::authenticated_projection_packets_keep_real_source_menus_and_refuse_stale_actions";
    const FLAG: &str = "SUNRISE_PROJECTION_INPUT_CASE";
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        NAME,
        FLAG,
        &["menus", "stale", "callback", "queued", "replica_pick"],
    ) {
        return;
    }
    let case = std::env::var(FLAG).unwrap();
    let policy = Arc::new(AtomicU8::new(0));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "input_probe",
            Box::new(Fixture(policy.clone())),
        )]
        .into(),
        || {
            with_guests(|game| {
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                let (handle, output) = setup(game);
                let original_position = game.lan.as_ref().unwrap().hub.guest(12).unwrap().position;
                if case == "menus" {
                    let open = request(game, handle, [10.5, 64.7, 10.5], Action::Use, 1);
                    send(game, open);
                    let window = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(12)
                        .unwrap()
                        .menu
                        .as_ref()
                        .unwrap()
                        .window;
                    // A physical-dimension pass cannot read its sentinel or close the source menu.
                    game.lan.as_mut().unwrap().hub.sync_containers(&game.world);
                    assert_eq!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .menu
                            .as_ref()
                            .unwrap()
                            .window,
                        window
                    );
                    game.lan_guest_packet(
                        12,
                        S::ContainerClick(play::ContainerClick {
                            container: window,
                            state: 0,
                            slot: 0,
                            button: 0,
                            input: 0,
                            changed: vec![],
                            carried: None,
                        }),
                    );
                    let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
                    assert_eq!(guest.carried.as_ref().unwrap().count, 7);
                    assert_eq!(guest.dimension, "minecraft:overworld");
                    assert_eq!(guest.position, original_position);
                    match game.world.block_entities.get(&(0, 64, 0)).unwrap() {
                        BlockEntity::Chest(slots) => {
                            assert_eq!(slots[0].as_ref().unwrap().count, 29)
                        }
                        _ => panic!(),
                    }
                    game.lan_guest_packet(
                        12,
                        S::ContainerClose(play::ContainerClose { container: window }),
                    );
                    assert!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .carried
                            .is_none()
                    );
                    assert_eq!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .inventory
                            .iter()
                            .flatten()
                            .map(|stack| stack.count)
                            .sum::<u32>(),
                        7
                    );
                    let open = request(game, handle, [11.5, 64.7, 10.5], Action::Use, 2);
                    send(game, open);
                    game.enter_level(Dim::from_id(handle.level().dimension_id()))
                        .unwrap();
                    if let BlockEntity::Furnace(f) =
                        game.world.block_entities.get_mut(&(1, 64, 0)).unwrap()
                    {
                        f.cooking_timer = 43;
                        f.items[2] = Some(ItemStack::new("minecraft:iron_ingot", 1));
                    }
                    game.lan.as_mut().unwrap().hub.sync_containers(&game.world);
                    assert!(packets(&output).iter().any(|packet|matches!(packet,C::ContainerSetData(data) if data.id==2 && data.value==43)));
                    assert!(packets(&output).iter().any(|packet|matches!(packet,C::ContainerSetSlot(slot) if slot.slot==2 && slot.item.is_some())));
                    game.enter_level(Dim::Overworld).unwrap();
                    // Keep a cursor at source-close to check the ordinary single-inventory return path.
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:coal", 3))
                    });
                    {
                        let _owner = owner();
                        let host: &mut dyn mod_api::ServerHost = game;
                        host.runtime_levels().unload("input_probe:source").unwrap();
                    }
                    game.tick_projection_menus();
                    assert!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .menu
                            .is_none()
                    );
                    assert!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .carried
                            .is_none()
                    );
                    assert!(!game.projection_menu_bound(12));
                    assert_eq!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .inventory
                            .iter()
                            .flatten()
                            .map(|stack| stack.count)
                            .sum::<u32>(),
                        10
                    );
                    assert_eq!(game.dim, Dim::Overworld);
                } else if case == "replica_pick" {
                    let (sample, membership, hash, revision) = {
                        let table = game.live_worlds.borrow();
                        let entry = &table.entries[&handle.generation()];
                        let world = game.native_level_world(handle.level()).unwrap();
                        (
                            entry.sample,
                            entry.membership,
                            super::super::state_hash(world, &entry.cells),
                            world.revision,
                        )
                    };
                    let pose = |pose: LiveWorldPose| sunrise_protocol::live_world_sync::Pose {
                        translation: pose.translation,
                        pivot: pose.pivot,
                        rotation: pose.rotation,
                    };
                    let snapshot = sunrise_protocol::live_world_sync::Message::Snapshot {
                        id: handle.generation(),
                        collider: 991,
                        boxes: 3,
                        source: handle.level().dimension_id().into(),
                        destination: "minecraft:overworld".into(),
                        cells: vec![[0, 64, 0], [1, 64, 0], [17, 64, 0]],
                        membership,
                        sample: sunrise_protocol::live_world_sync::Sample {
                            frame: 1,
                            source_revision: revision,
                            state_hash: hash,
                            previous: pose(sample.previous),
                            current: pose(sample.current),
                            revision: sample.revision,
                        },
                    };
                    game.remote_live_world_sync(
                        &sunrise_protocol::live_world_sync::encode(&snapshot).unwrap(),
                    )
                    .unwrap();
                    let eye = game.lan.as_ref().unwrap().hub.guest(12).unwrap().eye();
                    let direction = vector([10.5, 64.7, 10.5])
                        .subtract(vector(eye))
                        .normalize()
                        .components();
                    game.refresh_projection_pick(eye, direction, None);
                    assert_eq!(game.projection_pick().unwrap().hit.cell, [0, 64, 0]);
                    assert!(!game.projection_pick_outline(0.5).is_empty());
                    game.refresh_projection_pick(eye, direction, Some(1.0));
                    assert!(
                        game.projection_pick().is_none(),
                        "a nearer ordinary target wins"
                    );
                } else {
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.position = [27.5, 64.0, 13.0];
                        guest.inventory[0] = Some(ItemStack::new("minecraft:stone", 3));
                    });
                    let before = source_state(game, handle, [17, 64, 0]);
                    let valid = request(game, handle, [27.5, 64.7, 10.5], Action::StartDestroy, 3);
                    if case == "stale" {
                        for mut stale in [valid; 3].into_iter().enumerate() {
                            match stale.0 {
                                0 => stale.1.membership += 1,
                                1 => stale.1.pose_revision += 1,
                                _ => stale.1.source_revision += 1,
                            }
                            send(game, stale.1);
                            assert_eq!(source_state(game, handle, [17, 64, 0]), before);
                        }
                        game.lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |guest| guest.game_mode = 0);
                        send(game, valid);
                        assert_eq!(
                            source_state(game, handle, [17, 64, 0]),
                            before,
                            "survival never trusts an instant request"
                        );
                        game.lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |guest| guest.game_mode = 1);
                        let stone = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:stone")
                            .unwrap();
                        game.world.set(27, 64, 9, stone).unwrap();
                        game.lan_guest_packet(
                            12,
                            S::PlayerAction(play::PlayerAction {
                                action: 0,
                                pos: [27, 64, 9],
                                direction: 3,
                                sequence: 4,
                            }),
                        );
                        assert_eq!(
                            game.world.block(27, 64, 9),
                            stone,
                            "vanilla packets cannot destroy through a nearer projection"
                        );
                        {
                            let _owner = owner();
                            handle.retire(game).unwrap();
                        }
                        game.lan_guest_packet(
                            12,
                            S::PlayerAction(play::PlayerAction {
                                action: 0,
                                pos: [27, 64, 9],
                                direction: 3,
                                sequence: 5,
                            }),
                        );
                        assert!(
                            game.res.facts().is_air(game.world.block(27, 64, 9)),
                            "the same ordinary packet works with no projection"
                        );
                    } else {
                        policy.store(if case == "queued" { 2 } else { 1 }, Ordering::Relaxed);
                        send(game, valid);
                        assert_eq!(
                            source_state(game, handle, [17, 64, 0]),
                            before,
                            "callback invalidated prepared action"
                        );
                        assert_eq!(
                            game.lan.as_ref().unwrap().hub.guest(12).unwrap().inventory[0]
                                .as_ref()
                                .unwrap()
                                .count,
                            3
                        );
                    }
                }
            });
        },
    );
}

// Genuine negotiated guest requests and ordinary continuing menu/action contexts.
fn negotiated_probe_sender(player: mod_api::PlayerRef) -> sunrise_client::lan::PlayerSender {
    use sunrise_client::{
        lan::PlayerSender,
        raw_payloads::{Local, Side, Transport},
    };
    let local = Local::capture(Side::Server).unwrap();
    let transport = Transport::new(local.clone(), Side::Server);
    transport.accept(local.manifest.entries()).unwrap();
    transport.finish_configuration().unwrap();
    transport.bind_player(player);
    let (mut sender, _) = PlayerSender::payload_executor_test_sender(transport);
    sender.viewed_levels = sunrise_protocol::viewed_levels::Answer {
        revision: sunrise_protocol::viewed_levels::REVISION_4,
        max_levels: 4,
    };
    sender
}
fn inventory_count(game: &Game<'_>, item: &str) -> u32 {
    game.lan
        .as_ref()
        .unwrap()
        .hub
        .guest(12)
        .unwrap()
        .inventory
        .iter()
        .flatten()
        .filter(|stack| &*stack.item == item)
        .map(|stack| stack.count)
        .sum()
}
fn set_probe_actor(game: &mut Game<'_>, position: [f64; 3], count: u32) {
    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
        guest.position = position;
        guest.game_mode = 0;
        guest.inventory[0] = (count > 0).then(|| ItemStack::new("minecraft:stone", count));
    });
}

#[test]
#[ignore = "daily"]
fn projected_block_item_packets_keep_inventory_and_check_actual_physical_bodies() {
    const NAME: &str = "game::native_live_worlds::picking::tests::projected_block_item_packets_keep_inventory_and_check_actual_physical_bodies";
    const FLAG: &str = "SUNRISE_PROJECTION_PLACEMENT_CASE";
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        NAME,
        FLAG,
        &[
            "valid",
            "own",
            "other",
            "rotated_own",
            "rotated_other",
            "inventory",
            "selection",
            "close",
            "retire",
            "old_pose",
            "source_guest",
            "local_coordinate_alias",
        ],
    ) {
        return;
    }
    let case = std::env::var(FLAG).unwrap();
    let policy = Arc::new(AtomicU8::new(0));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "input_probe",
            Box::new(Fixture(policy.clone())),
        )]
        .into(),
        || {
            with_guests(|game| {
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                let (mut handle, _output) = setup(game);
                let rotated = case.starts_with("rotated_");
                if rotated || case == "local_coordinate_alias" {
                    let _owner = owner();
                    let source = handle.level();
                    handle.retire(game).unwrap();
                    let mut pose = LiveWorldPose::at([10.0, 0.0, 10.0]);
                    let half = std::f64::consts::FRAC_PI_4;
                    if rotated {
                        pose.rotation = [0.0, half.sin(), 0.0, half.cos()];
                    } else {
                        pose.translation = [0.0, 0.0, -2.0];
                    }
                    let destination = game.level("minecraft:overworld").unwrap();
                    handle = source
                        .project(
                            game,
                            destination,
                            &[[0, 64, 0], [1, 64, 0], [17, 64, 0]],
                            pose,
                            LiveWorldLimits::default(),
                        )
                        .unwrap();
                }
                let own = case == "own" || case == "rotated_own";
                let actor = if case == "local_coordinate_alias" {
                    [17.5, 64.0, 1.5]
                } else if rotated {
                    [if own { 11.8 } else { 13.0 }, 64.0, -7.5]
                } else {
                    [27.5, 64.0, if own { 11.8 } else { 13.0 }]
                };
                set_probe_actor(game, actor, 3);
                if matches!(case.as_str(), "other" | "rotated_other" | "source_guest") {
                    let sender = negotiated_probe_sender(game.player(13).unwrap());
                    let blocker = if case == "source_guest" {
                        [17.5, 64.0, 1.5]
                    } else if rotated {
                        [11.5, 64.0, -7.5]
                    } else {
                        [27.5, 64.0, 11.5]
                    };
                    game.lan.as_mut().unwrap().hub.guests.update(1, |guest| {
                        guest.dimension = if case == "source_guest" {
                            handle.level().dimension_id().into()
                        } else {
                            "minecraft:overworld".into()
                        };
                        guest.position = blocker;
                        guest.sender = sender;
                        guest.loaded = true;
                    });
                }
                let target = if case == "local_coordinate_alias" {
                    [17.5, 64.2, -1.5]
                } else if rotated {
                    [10.5, 64.2, -7.5]
                } else {
                    [27.5, 64.2, 10.5]
                };
                let input = request(game, handle, target, Action::Use, 10);
                assert_eq!(input.cell, [17, 64, 0]);
                assert_eq!(input.face, 3, "ordinary source +Z face chooses [17,64,1]");
                match case.as_str() {
                    "inventory" => policy.store(5, Ordering::Relaxed),
                    "selection" => policy.store(8, Ordering::Relaxed),
                    "close" => policy.store(6, Ordering::Relaxed),
                    "retire" => policy.store(7, Ordering::Relaxed),
                    "old_pose" => {
                        let _owner = owner();
                        game.live_world_begin_tick();
                        handle
                            .publish_pose(game, 0, LiveWorldPose::at([10.1, 0.0, 10.0]))
                            .unwrap();
                    }
                    _ => {}
                }
                send(game, input);
                let world = game.native_level_world(handle.level()).unwrap();
                let placed = !game.res.facts().is_air(world.block(17, 64, 1));
                if matches!(case.as_str(), "valid" | "local_coordinate_alias") {
                    assert!(
                        placed,
                        "actual held block request must reach ordinary source placement"
                    );
                    assert_eq!(
                        inventory_count(game, "minecraft:stone"),
                        2,
                        "ordinary survival use consumes one, mining remains separately refused"
                    );
                    assert!(
                        game.live_worlds.borrow().entries[&handle.generation()]
                            .cells
                            .contains(&[17, 64, 1])
                    );
                } else {
                    assert!(
                        !placed,
                        "real physical obstruction or stale callback must refuse before source mutation: {case}"
                    );
                    assert_eq!(
                        inventory_count(game, "minecraft:stone"),
                        3,
                        "failed attempt cannot consume or retain callback inventory mutation: {case}"
                    );
                }
                if case == "selection" {
                    // Existing set_selected_slot is an explicit non-journaled input service.
                    // Its chosen slot remains; the old-hand placement must still be refused.
                    assert_eq!(
                        game.lan.as_ref().unwrap().hub.guest(12).unwrap().selected,
                        1
                    );
                }
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guest(12).unwrap().position,
                    actor,
                    "body stays in physical frame"
                );
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guest(12).unwrap().dimension,
                    "minecraft:overworld"
                );
                // Move the SAME actual blocker away and retry, so a rejection cannot be explained
                // by unsupported item, source readiness, negotiation or a different fixture setup.
                if matches!(
                    case.as_str(),
                    "own" | "other" | "rotated_own" | "rotated_other" | "source_guest"
                ) {
                    if own {
                        set_probe_actor(
                            game,
                            if rotated {
                                [13.0, 64.0, -7.5]
                            } else {
                                [27.5, 64.0, 13.0]
                            },
                            3,
                        );
                    } else {
                        game.lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(1, |guest| guest.position = [20.0, 70.0, 20.0]);
                    }
                    let retry = request(game, handle, target, Action::Use, 11);
                    send(game, retry);
                    assert!(
                        !game.res.facts().is_air(
                            game.native_level_world(handle.level())
                                .unwrap()
                                .block(17, 64, 1)
                        )
                    );
                    assert_eq!(inventory_count(game, "minecraft:stone"), 2);
                }
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn projected_menu_callbacks_and_continuing_cleanup_conserve_cursor_and_openers() {
    const NAME: &str = "game::native_live_worlds::picking::tests::projected_menu_callbacks_and_continuing_cleanup_conserve_cursor_and_openers";
    const FLAG: &str = "SUNRISE_PROJECTION_MENU_ADVERSE_CASE";
    if crate::game::native_entity_refs::tests::run_fresh_cases(
        NAME,
        FLAG,
        &[
            "outer_transaction",
            "inventory",
            "selection",
            "queued",
            "retire_before_open",
            "reach",
            "retire_open",
            "source_close",
            "unnegotiated",
        ],
    ) {
        return;
    }
    let case = std::env::var(FLAG).unwrap();
    let policy = Arc::new(AtomicU8::new(0));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "input_probe",
            Box::new(Fixture(policy.clone())),
        )]
        .into(),
        || {
            with_guests(|game| {
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                let (handle, output) = setup(game);
                if case == "outer_transaction" {
                    set_probe_actor(game, [10.5, 64.0, 13.0], 3);
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:gold_ingot", 2));
                    });
                    let player = game.player(12).unwrap();
                    let before_inventory = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(12)
                        .unwrap()
                        .inventory
                        .clone();
                    let before_cursor = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(12)
                        .unwrap()
                        .carried
                        .clone();
                    let _owner = owner();
                    let eye = game.lan.as_ref().unwrap().hub.guest(12).unwrap().eye();
                    let hit = handle.clip(game, eye, [10.5, 64.7, 10.5]).unwrap().unwrap();
                    game.enter_level(Dim::from_id(handle.level().dimension_id()))
                        .unwrap();
                    let frame = game.open_storage_transaction();
                    assert_eq!(
                        handle.use_block(game, player, hit, mod_api::Hand::Main),
                        Err(Error::Busy)
                    );
                    assert!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .menu
                            .is_none()
                    );
                    assert_eq!(
                        game.lan.as_ref().unwrap().hub.guest(12).unwrap().inventory,
                        before_inventory
                    );
                    assert_eq!(
                        game.lan.as_ref().unwrap().hub.guest(12).unwrap().carried,
                        before_cursor
                    );
                    assert!(!game.lid_openers.contains_key(&(0, 64, 0)));
                    let entity = game.world.block_entities.get(&(0, 64, 0)).unwrap();
                    assert_eq!(entity.slots()[0].as_ref().unwrap().count, 7);
                    assert!(
                        !packets(&output)
                            .iter()
                            .any(|packet| matches!(packet, C::OpenScreen(_)))
                    );
                    let _ = game.close_storage_frame(frame, false);
                    game.enter_level(Dim::Overworld).unwrap();
                    // Return the nonempty inventory cursor through its actual ordinary close.
                    game.lan_guest_packet(
                        12,
                        S::ContainerClose(play::ContainerClose { container: 0 }),
                    );
                    assert!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .carried
                            .is_none()
                    );
                    assert_eq!(inventory_count(game, "minecraft:gold_ingot"), 2);
                    assert_eq!(
                        handle.use_block(game, player, hit, mod_api::Hand::Main),
                        Ok(true),
                        "same real target works once its caller-held savepoint has settled"
                    );
                    let window = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(12)
                        .unwrap()
                        .menu
                        .as_ref()
                        .unwrap()
                        .window;
                    game.lan_guest_packet(
                        12,
                        S::ContainerClose(play::ContainerClose { container: window }),
                    );
                    assert!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .menu
                            .is_none()
                    );
                    assert_eq!(inventory_count(game, "minecraft:stone"), 3);
                    assert_eq!(inventory_count(game, "minecraft:gold_ingot"), 2);
                    game.enter_level(Dim::from_id(handle.level().dimension_id()))
                        .unwrap();
                    assert!(!game.lid_openers.contains_key(&(0, 64, 0)));
                    assert_eq!(
                        game.world.block_entities.get(&(0, 64, 0)).unwrap().slots()[0]
                            .as_ref()
                            .unwrap()
                            .count,
                        7
                    );
                    return;
                }
                let preopen = matches!(
                    case.as_str(),
                    "inventory" | "selection" | "queued" | "retire_before_open" | "unnegotiated"
                );
                if preopen {
                    set_probe_actor(game, [10.5, 64.0, 13.0], 3);
                }
                match case.as_str() {
                    "inventory" => policy.store(10, Ordering::Relaxed),
                    "selection" => policy.store(13, Ordering::Relaxed),
                    "queued" => policy.store(11, Ordering::Relaxed),
                    "retire_before_open" => policy.store(12, Ordering::Relaxed),
                    "unnegotiated" => game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(12)
                        .unwrap()
                        .sender
                        .payload_transport()
                        .begin_configuration(),
                    _ => {}
                }
                let input = request(game, handle, [10.5, 64.7, 10.5], Action::Use, 20);
                send(game, input);
                if preopen {
                    assert!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .menu
                            .is_none()
                    );
                    assert_eq!(inventory_count(game, "minecraft:stone"), 3);
                    if case == "selection" {
                        assert_eq!(
                            game.lan.as_ref().unwrap().hub.guest(12).unwrap().selected,
                            1
                        );
                    }
                    assert!(
                        !packets(&output)
                            .iter()
                            .any(|p| matches!(p, C::OpenScreen(_))),
                        "no tentative screen before fresh admission"
                    );
                } else {
                    let window = game
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(12)
                        .unwrap()
                        .menu
                        .as_ref()
                        .unwrap()
                        .window;
                    assert_eq!(
                        game.lan.as_ref().unwrap().play[&12]
                            .stats
                            .get("custom", "minecraft:open_chest"),
                        1,
                        "ordinary use stat is emitted once; handoff must not replay use"
                    );
                    game.with_level(|_, level, _| {
                        assert!(
                            !level.open_containers.contains(&(0, 64, 0)),
                            "parent level must not count source-local coordinate"
                        )
                    });
                    game.enter_level(Dim::from_id(handle.level().dimension_id()))
                        .unwrap();
                    game.with_level(|_, level, _| {
                        assert_eq!(
                            level
                                .open_containers
                                .iter()
                                .filter(|&&pos| pos == (0, 64, 0))
                                .count(),
                            1,
                            "ordinary source opener recount sees the real bound menu once"
                        )
                    });
                    assert_eq!(
                        game.lid_openers.get(&(0, 64, 0)),
                        Some(&1),
                        "exactly one ordinary opener"
                    );
                    game.enter_level(Dim::Overworld).unwrap();
                    game.lan_guest_packet(
                        12,
                        S::ContainerClick(play::ContainerClick {
                            container: window,
                            state: 0,
                            slot: 0,
                            button: 0,
                            input: 0,
                            changed: vec![],
                            carried: None,
                        }),
                    );
                    assert_eq!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .carried
                            .as_ref()
                            .unwrap()
                            .count,
                        7
                    );
                    match case.as_str() {
                        "reach" => game
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |guest| guest.position = [100.0, 64.0, 100.0]),
                        "retire_open" => {
                            let _owner = owner();
                            handle.retire(game).unwrap();
                        }
                        "source_close" => {
                            let _owner = owner();
                            (game as &mut dyn mod_api::ServerHost)
                                .runtime_levels()
                                .unload("input_probe:source")
                                .unwrap();
                        }
                        _ => unreachable!(),
                    }
                    game.tick_projection_menus();
                    assert!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .menu
                            .is_none()
                    );
                    assert!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(12)
                            .unwrap()
                            .carried
                            .is_none()
                    );
                    assert_eq!(
                        inventory_count(game, "minecraft:stone"),
                        7,
                        "one ordinary return path conserves cursor"
                    );
                    assert!(!game.projection_menu_bound(12));
                    assert!(
                        packets(&output)
                            .iter()
                            .any(|p| matches!(p,C::ContainerClose(c) if c.container==window))
                    );
                }
                game.enter_level(Dim::from_id(handle.level().dimension_id()))
                    .unwrap();
                assert!(
                    !game.lid_openers.contains_key(&(0, 64, 0)),
                    "failed/closed menu releases source opener once"
                );
                game.enter_level(Dim::Overworld).unwrap();
            });
        },
    );
}
