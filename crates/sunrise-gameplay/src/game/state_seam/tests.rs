use super::*;
use crate::game::advancements::block_use_recording::{Recording, SEEN};
use crate::game::native_entity_refs::tests::{
    fixture_root, prepare_entity_ticking_neighborhood, run_fresh_cases, run_with_game_stack,
    with_game_role,
};
use crate::game::native_guest_turn_tests::open_fixture_hub;
use mod_api::{
    BlockPart, CommandSource, Flow, Hooks, HostSide, InteractionResult, ModEntry, PermissionLevel,
    Registrar, RegistrationContext, ServerHost, TriState,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use sunrise_core::{position::ChunkPos, vanilla_bytes::Writer};
use sunrise_gameplay_common::game::{
    GameGameExt1 as _, TryIntoApi as _, chat_common::ChatCommonGameExt1 as _, end_state::END_SPAWN,
    execution_role::ExecutionRole, lan_actions::TurnRefused,
    server_side::server_travel::credits_respawn_tests::placed_guest, tables::WorldTablesExt as _,
};
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::{
    buf::{Codec, Reader},
    packets::{
        configuration::CustomPayload,
        play::{self, Clientbound as C, Serverbound as S},
    },
};
use sunrise_sim::{
    lan::{PlayerSender, hub::game_event},
    raw_payloads::{Side, Transport, connection::record_overhead, negotiated},
};

const USE_CHANNEL: &str = sunrise_protocol::packets::custom_payload::CAULDRON_USE_CHANNEL;
const A: u32 = 12;
const A_UUID: u128 = 342;
const B: u32 = 13;
const B_UUID: u128 = 343;
const HIT: [i32; 3] = [3, 64, 4];
const BESIDE: [i32; 3] = [3, 65, 4];

type TicketObservations = Vec<(bool, bool)>;

thread_local! {
    static ARRIVAL_TICKETS: std::cell::RefCell<Option<TicketObservations>> =
        const { std::cell::RefCell::new(None) };
    static RESUMES: std::cell::RefCell<Vec<(ActorInLevel, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
    static COMMAND_ACTORS: std::cell::RefCell<Vec<Option<PlayerId>>> = const { std::cell::RefCell::new(Vec::new()) };
    static SOUND_PRESENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

struct TicketRecording;

impl TicketRecording {
    fn start() -> Self {
        ARRIVAL_TICKETS.with_borrow_mut(|observations| *observations = Some(Vec::new()));
        Self
    }

    fn finish(self) -> TicketObservations {
        ARRIVAL_TICKETS.with_borrow_mut(Option::take).unwrap()
    }
}

impl Drop for TicketRecording {
    fn drop(&mut self) {
        ARRIVAL_TICKETS.with_borrow_mut(|observations| *observations = None);
    }
}

pub(super) fn record_resume(actor: ActorInLevel, valid: bool) {
    RESUMES.with_borrow_mut(|observations| observations.push((actor, valid)));
}

pub(in crate::game) fn record_command_actor(actor: Option<PlayerId>) {
    COMMAND_ACTORS.with_borrow_mut(|observations| observations.push(actor));
}

pub(in crate::game) fn record_pending_placement_sounds(
    game: &ServerGame<'_>,
    conn: u32,
    heard: usize,
) {
    let placer = game
        .server
        .lan
        .as_ref()
        .and_then(|host| host.hub.guest(conn));
    SOUND_PRESENT.set(placer.is_some_and(|placer| {
        game.server
            .pending_sounds
            .iter()
            .skip(heard)
            .any(|sound| sound.except_entity == Some(placer.entity_id as u32))
    }));
}

#[test]
fn guest_slot_reacquires_by_identity_and_rejects_reuse() {
    let mut guests = GuestList::default();
    let (a, _rx) = placed_guest(A, Dim::Overworld, [0.; 3]);
    let (b, _rx) = placed_guest(B, Dim::Overworld, [0.; 3]);
    let actor = PlayerId::guest(&a);
    guests.push(b);
    guests.push(a);
    assert_eq!(actor.live_slot(&guests), Some(1));
    guests.remove(0);
    assert_eq!(actor.live_slot(&guests), Some(0));
    guests.remove(0);
    let (replacement, _rx) = placed_guest(A, Dim::Overworld, [0.; 3]);
    guests.push(replacement);
    assert_eq!(actor.live_slot(&guests), None);
}

pub(in crate::game) fn record_arrival_ticket(game: &ServerGame<'_>, changed: bool) {
    ARRIVAL_TICKETS.with_borrow_mut(|observations| {
        if let Some(observations) = observations {
            let [x, _, z] = END_SPAWN;
            let present = game
                .server
                .chunk_ticking
                .tickets
                .portal_tickets()
                .iter()
                .any(|ticket| ticket.chunk == (x >> 4, z >> 4));
            observations.push((changed, present));
        }
    });
}

fn correction_order(packets: &[C], sequence: i32, expected: [bool; 2], receipt_first: bool) {
    let receipt = packet_index(packets, |p| receipt(p, sequence) == Some(expected));
    let hit = packet_index(packets, |p| matches!(p, C::BlockUpdate(p) if p.pos == HIT));
    let beside = packet_index(
        packets,
        |p| matches!(p, C::BlockUpdate(p) if p.pos == BESIDE),
    );
    assert!(hit < beside);
    if receipt_first {
        assert!(receipt < hit);
    } else {
        let warning = packet_index(packets, |p| matches!(p, C::SystemChat(p) if p.overlay));
        assert!(warning < hit && beside < receipt);
    }
}

fn wire_ack(game: &mut ServerGame<'_>, out: &impl Fn() -> Vec<C>, sequence: i32) {
    let before = out().len();
    game.tick_lan(true);
    let after = out();
    let ack = packet_index(
        &after,
        |p| matches!(p, C::BlockChangedAck(p) if p.sequence == sequence),
    );
    assert!(before <= ack);
}

fn add_player(game: &mut ServerGame<'_>, conn: u32) -> impl Fn() -> Vec<C> + use<> {
    let (mut guest, _) = placed_guest(conn, Dim::Overworld, [3.5, 64.0, 6.0]);
    let transport = Transport::capture(Side::Server, record_overhead(&[])).unwrap();
    transport
        .accept(transport.local.manifest.entries())
        .unwrap();
    transport.finish_configuration().unwrap();
    assert!(negotiated(&transport, USE_CHANNEL));
    let (sender, inspect) = PlayerSender::payload_executor_test_sender(transport);
    guest.sender = sender;
    game.server.next_entity = game.server.next_entity.max(conn);
    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
    move || {
        inspect()
            .iter()
            .filter_map(|body| {
                let mut reader = Reader::new(body);
                C::decode(reader.varint().ok()?, reader.rest()).ok()
            })
            .collect()
    }
}

fn use_on(game: &mut ServerGame<'_>, pos: [i32; 3], sequence: i32) {
    let click = play::UseItemOn {
        hand: 0,
        pos,
        face: 1,
        cursor: [0.5, 1.0, 0.5],
        inside: false,
        world_border: false,
        sequence,
    };
    let mut body = Writer::new();
    body.i64(i64::from(sequence));
    play::UseItemOn::write(&click, &mut body);
    game.lan_guest_packet(
        A,
        S::CustomPayload(CustomPayload {
            channel: USE_CHANNEL.into(),
            data: body.data,
        }),
    );
    let player = game.server.lan.as_ref().unwrap().hub.guest(A).unwrap();
    assert_eq!(player.ack_up_to.get(), sequence);
}

fn receipt(packet: &C, sequence: i32) -> Option<[bool; 2]> {
    let C::CustomPayload(p) = packet else {
        return None;
    };
    if p.channel != USE_CHANNEL {
        return None;
    }
    let mut r = Reader::new(&p.data);
    if r.i64().ok()? != i64::from(sequence) {
        return None;
    }
    let outcome = [r.bool().unwrap(), r.bool().unwrap()];
    assert_eq!(r.remaining(), 0);
    Some(outcome)
}

fn packet_index(packets: &[C], matches: impl FnMut(&C) -> bool) -> usize {
    let mut matches = matches;
    let mut indexes = packets
        .iter()
        .enumerate()
        .filter(|(_, packet)| matches(packet));
    let first = indexes.next().expect("expected packet").0;
    assert!(indexes.next().is_none(), "duplicate matching packet");
    first
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Travel {
    Allow,
    Refuse,
    Disconnect,
    DisconnectAtPlatform,
    ReopenDestination,
}

struct TravelListener {
    mode: Travel,
    dedicated: bool,
    events: Arc<Mutex<Vec<&'static str>>>,
}

#[mod_api::__subscriptions]
impl Hooks for TravelListener {
    fn server_started(&self, _: &mut dyn ServerHost, event: &mut mod_api::ServerStartedEvent) {
        assert_eq!(event.dedicated, self.dedicated);
        self.events.lock().unwrap().push("authority");
    }

    fn entity_travel_to_dimension(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::EntityTravelToDimensionEvent<'_>,
    ) -> Flow {
        assert_eq!(event.cause, "end_portal");
        assert_eq!(event.dimension, Dim::End.id());
        assert_eq!(host.dimension(), Dim::Overworld.id());
        let player = host.player(A_UUID).unwrap();
        assert_eq!(event.entity, player.entity());
        assert_eq!(
            player.level(host).unwrap().dimension(),
            Dim::Overworld.key()
        );
        self.events.lock().unwrap().push("travel");
        match self.mode {
            Travel::Allow | Travel::DisconnectAtPlatform | Travel::ReopenDestination => {
                Flow::Continue
            }
            Travel::Refuse => Flow::Cancel,
            Travel::Disconnect => {
                assert!(
                    player
                        .disconnect(host, mod_api::Text::literal("Leaving"))
                        .unwrap()
                );
                Flow::Continue
            }
        }
    }

    fn level_load(&self, host: &mut dyn ServerHost, event: &mut mod_api::LevelLoadEvent<'_>) {
        if event.dimension == Dim::End.id() {
            assert_eq!(host.dimension(), Dim::End.id());
            assert!(host.level(event.dimension).unwrap().in_place(host));
            self.events.lock().unwrap().push("load");
        }
    }

    fn portal_create(&self, host: &mut dyn ServerHost, event: &mut mod_api::PortalCreateEvent<'_>) {
        assert_eq!(event.dimension, Dim::End.id());
        let [x, y, z] = END_SPAWN;
        assert_eq!(event.pos, [x, y - 1, z]);
        let level = host.level(event.dimension).unwrap();
        let block = level.block_state(host, [x, y - 2, z]).unwrap();
        assert_eq!(block.block().id(host), Some("minecraft:obsidian"));
        assert_eq!(host.players().acting_player().unwrap().uuid(), A_UUID);
        let cleared = level.block_state(host, [x, y - 1, z]).unwrap();
        assert_eq!(cleared.block().id(host), Some("minecraft:air"));
        self.events.lock().unwrap().push("platform");
        if self.mode == Travel::DisconnectAtPlatform {
            let player = host.players().acting_player().unwrap();
            assert!(
                player
                    .disconnect(host, mod_api::Text::literal("Leaving at the platform"))
                    .unwrap()
            );
        }
    }

    fn player_changed_dimension(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::PlayerChangedDimensionEvent<'_>,
    ) {
        assert_eq!((event.from, event.to), (Dim::Overworld.id(), Dim::End.id()));
        assert_eq!(event.player, A_UUID);
        assert_eq!(host.dimension(), Dim::End.id());
        assert_eq!(host.players().acting_player().unwrap().uuid(), A_UUID);
        self.events.lock().unwrap().push("changed");
    }
}

fn spectator_arrival(role: ExecutionRole, mode: Travel) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "observe-travel",
        Box::new(TravelListener {
            mode,
            dedicated: role == ExecutionRole::DedicatedServer,
            events: events.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game_role(fixture_root(), role, |game| {
            let scratch = tempfile::tempdir().unwrap();
            game.server.world_root = scratch.path().to_path_buf();
            if role == ExecutionRole::DedicatedServer {
                game.options.dedicated = Some(Vec::new());
                game.common
                    .native_lifecycle
                    .begin_open(Some(mod_api::Authority::DedicatedServer));
            }
            game.native_level_opened(Dim::Overworld);
            game.server_lifecycle_world_ready();
            assert_eq!(std::mem::take(&mut *events.lock().unwrap()), ["authority"]);
            open_fixture_hub(game);
            let out = add_player(game, A);
            let watcher = add_player(game, B);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(A, |guest| {
                    guest.game_mode = 3;
                })
                .unwrap();
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            game.set_state_named(HIT, "minecraft:end_portal", &[])
                .unwrap();
            assert!(game.server.levels.0.parked_world(Dim::End).is_none());

            use_on(game, HIT, 7);
            assert_eq!(game.dim, Dim::Overworld);
            if mode == Travel::Refuse {
                assert_eq!(*events.lock().unwrap(), ["travel"]);
                assert!(!game.has_pending_travel(A));
                assert!(game.server.levels.0.parked_world(Dim::End).is_none());
                let packets = out();
                packet_index(&packets, |p| receipt(p, 7) == Some([false, false]));
                assert!(
                    !packets
                        .iter()
                        .any(|p| matches!(p, C::Respawn(_) | C::PlayerPosition(_)))
                );
                return;
            }
            if mode == Travel::Disconnect {
                let guest = game.server.lan.as_ref().unwrap().hub.guest(A).unwrap();
                assert!(guest.disconnect_pending);
                assert_eq!(guest.dimension, Dim::End.id());
                assert!(guest.landing && game.has_pending_travel(A));
                assert_eq!(*events.lock().unwrap(), ["travel", "load"]);
                assert!(!out().iter().any(|p| receipt(p, 7).is_some()));
                return;
            }

            assert_eq!(*events.lock().unwrap(), ["travel", "load"]);
            let guest = game.server.lan.as_ref().unwrap().hub.guest(A).unwrap();
            assert_eq!(guest.dimension, Dim::End.id());
            assert!(guest.landing && game.has_pending_travel(A));
            let before = out();
            let respawn = packet_index(&before, |p| matches!(p, C::Respawn(_)));
            let replied = packet_index(&before, |p| receipt(p, 7) == Some([false, false]));
            let commands = packet_index(&before, |p| matches!(p, C::Commands(_)));
            let abilities = packet_index(&before, |p| matches!(p, C::PlayerAbilities(_)));
            let cache = packet_index(&before, |p| matches!(p, C::SetChunkCacheCenter(_)));
            assert!(
                respawn < commands && commands < abilities && abilities < cache && cache < replied
            );
            assert!(!before.iter().any(|p| matches!(p, C::PlayerPosition(_))));

            game.enter_level(Dim::End).unwrap();
            if mode == Travel::ReopenDestination {
                let index = game.guest_index(A).unwrap();
                let player =
                    PlayerId::guest(game.server.lan.as_ref().unwrap().hub.guest(A).unwrap());
                let level = LevelId::loaded(game, Dim::End).unwrap();
                let actor = ActorInLevel { player, level };
                assert!(actor.view_at(game, index).is_some());
                game.native_level_closed(Dim::End);
                game.native_level_opened(Dim::End);
                assert!(actor.view_at(game, index).is_none());
                assert!(game.has_pending_travel(A));
            }
            let [x, y, z] = END_SPAWN;
            let (cx, cz) = (x >> 4, z >> 4);
            for dx in -2..=2 {
                for dz in -2..=2 {
                    game.server
                        .streaming
                        .insert_remote(&mut game.common.world, Chunk::empty(cx + dx, cz + dz));
                }
            }
            game.set_state_named([x, y - 2, z], "minecraft:stone", &[])
                .unwrap();
            game.set_state_named([x, y - 1, z], "minecraft:dirt", &[])
                .unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            assert!(game.levels_to_tick().contains(&Dim::End));
            RESUMES.with_borrow_mut(Vec::clear);
            let recording = TicketRecording::start();
            game.tick_server(true).unwrap();
            let tickets = recording.finish();
            RESUMES.with_borrow(|resumes| {
                let expected_valid = mode != Travel::ReopenDestination;
                assert!(
                    resumes
                        .iter()
                        .any(|(actor, valid)| actor.level.dim == Dim::End
                            && *valid == expected_valid)
                );
                if mode == Travel::DisconnectAtPlatform {
                    assert!(
                        resumes
                            .iter()
                            .any(|(actor, valid)| actor.level.dim == Dim::End && !valid)
                    );
                }
            });
            assert_eq!(tickets, [(false, false), (true, true)]);
            game.enter_level(Dim::End).unwrap();
            let expected: &[_] = if mode == Travel::ReopenDestination {
                &["travel", "load", "load", "platform", "changed"]
            } else {
                &["travel", "load", "platform", "changed"]
            };
            assert_eq!(*events.lock().unwrap(), expected);
            assert_eq!(&*game.block_name([x, y - 2, z]), "minecraft:obsidian");
            assert_eq!(&*game.block_name([x, y - 1, z]), "minecraft:air");
            assert!(
                game.server
                    .chunk_ticking
                    .tickets
                    .portal_tickets()
                    .iter()
                    .any(|ticket| ticket.chunk == (cx, cz))
            );
            assert!(!game.has_pending_travel(A));
            if mode == Travel::DisconnectAtPlatform {
                let guest = game.server.lan.as_ref().unwrap().hub.guest(A).unwrap();
                assert!(guest.disconnect_pending);
                assert!(!guest.landing);
                return;
            }
            let packets = out();
            let position = packet_index(&packets, |p| matches!(p, C::PlayerPosition(_)));
            let ready = packet_index(
                &packets,
                |p| matches!(p, C::GameEvent(e) if e.event == game_event::LEVEL_CHUNKS_LOAD_START),
            );
            let ack = packet_index(
                &packets,
                |p| matches!(p, C::BlockChangedAck(p) if p.sequence == 7),
            );
            let sound = packet_index(
                &packets,
                |p| matches!(p, C::LevelEvent(event) if event.kind == 1032),
            );
            assert!(replied < ack && ack < position && position < ready && ready < sound);
            assert_eq!(
                packet_index(&packets, |p| matches!(p, C::Respawn(_))),
                respawn
            );
            assert!(
                !watcher()
                    .iter()
                    .any(|p| matches!(p, C::Respawn(_) | C::PlayerPosition(_))
                        || receipt(p, 7).is_some())
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn spectator_use_packet_opens_and_lands_in_the_end() {
    if run_fresh_cases(
        "game::state_seam::tests::spectator_use_packet_opens_and_lands_in_the_end",
        "SUNRISE_SEAM_SPECTATOR_CHILD",
        &["travel"],
    ) {
        return;
    }
    run_with_game_stack(|| {
        for role in [
            ExecutionRole::IntegratedServer,
            ExecutionRole::DedicatedServer,
        ] {
            for mode in [
                Travel::Allow,
                Travel::Refuse,
                Travel::Disconnect,
                Travel::DisconnectAtPlatform,
                Travel::ReopenDestination,
            ] {
                spectator_arrival(role, mode);
            }
        }
    });
}

struct BlockListener {
    suppress: Arc<AtomicBool>,
    disconnect: Arc<AtomicBool>,
    events: Arc<Mutex<Vec<&'static str>>>,
}

struct StonePart(Arc<Mutex<Vec<&'static str>>>);

#[mod_api::sunrise_part]
impl BlockPart for StonePart {
    fn use_item_on(
        &self,
        _: HostSide<'_>,
        event: &mut mod_api::UseItemOnEvent<'_>,
    ) -> Option<InteractionResult> {
        assert_eq!(event.click.pos, HIT);
        assert_eq!(event.click.player, A_UUID);
        assert!(event.click.stack.is_some());
        self.0.lock().unwrap().push("block");
        None
    }
}

#[mod_api::__subscriptions]
impl Hooks for BlockListener {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        let stone = registrar.block_key("minecraft:stone");
        registrar.block_part(stone, StonePart(self.events.clone()));
    }

    fn register_commands(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::RegisterCommandsEvent<'_>,
    ) {
        let events = self.events.clone();
        event
            .commands
            .push(
                mod_api::Command::new("checknested").executes(move |host, source, command| {
                    let CommandSource::Player { player, permission } = *source else {
                        panic!("expected the nested command's original player");
                    };
                    assert_eq!(
                        (player.uuid(), permission),
                        (B_UUID, PermissionLevel::Owners)
                    );
                    assert_eq!(command.position, [4.5, 64.0, 6.5]);
                    assert_eq!(command.dimension, Dim::Nether.key());
                    assert_eq!(host.players().acting_player(), Some(player));
                    let displaced = mod_api::__private::game::player_ref(
                        mod_api::EntityRef::new(player.entity().epoch(), A),
                        A_UUID,
                    );
                    assert!(!displaced.entity().run_command(
                        host,
                        "checknested",
                        PermissionLevel::Owners,
                        true
                    ));
                    events.lock().unwrap().push("nested");
                    Ok(1)
                }),
            );
    }

    fn right_click_block(
        &self,
        host: HostSide<'_>,
        event: &mut mod_api::RightClickBlockEvent<'_>,
    ) -> Flow {
        let HostSide::Server(host) = host else {
            panic!("the click is on the server");
        };
        assert_eq!(event.click.pos, HIT);
        assert_eq!(event.click.player, A_UUID);
        let actor = host.players().acting_player().unwrap();
        assert_eq!(actor.uuid(), A_UUID);
        let parked = host.level(Dim::Nether.id()).unwrap();
        assert_eq!(
            parked.block_state(host, HIT).unwrap().block().id(host),
            Some("minecraft:gold_block")
        );
        self.events.lock().unwrap().push("click");
        let b = host.player(B_UUID).unwrap();
        assert!(b.entity().run_command(
            host,
            "execute in minecraft:the_nether positioned 4 64 6 run checknested",
            PermissionLevel::Owners,
            true
        ));
        assert_eq!(host.players().acting_player(), Some(actor));
        assert_eq!(actor.level(host).unwrap().dimension(), Dim::Overworld.key());
        assert_eq!(host.level(Dim::Nether.id()), Some(parked));
        self.events.lock().unwrap().push("restored");
        if self.suppress.load(Ordering::Relaxed) {
            *event.use_item = TriState::False;
        } else {
            host.living(actor.entity())
                .unwrap()
                .set_hand_item(
                    host,
                    mod_api::Hand::Main,
                    Some(ItemStack::new("minecraft:dirt", 5).try_into_api().unwrap()),
                )
                .unwrap();
        }
        if self.disconnect.load(Ordering::Relaxed) {
            assert!(
                actor
                    .disconnect(host, mod_api::Text::literal("Leaving during use"))
                    .unwrap()
            );
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn placement_packet_restores_actor_and_source_and_corrects_prediction() {
    if run_fresh_cases(
        "game::state_seam::tests::placement_packet_restores_actor_and_source_and_corrects_prediction",
        "SUNRISE_SEAM_PLACEMENT_CHILD",
        &["placement"],
    ) {
        return;
    }
    run_with_game_stack(|| {
        let (events, suppress) = (
            Arc::new(Mutex::new(Vec::new())),
            Arc::new(AtomicBool::new(false)),
        );
        let disconnect = Arc::new(AtomicBool::new(false));
        let mods = vec![ModEntry::new(
            "observe-placement",
            Box::new(BlockListener {
                suppress: suppress.clone(),
                disconnect: disconnect.clone(),
                events: events.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game_role(fixture_root(), ExecutionRole::IntegratedServer, |game| {
                let scratch = tempfile::tempdir().unwrap();
                game.server.world_root = scratch.path().to_path_buf();
                game.ensure_datapacks();
                game.build_registries();
                open_fixture_hub(game);
                game.register_native_commands();
                let out = add_player(game, A);
                let watcher = add_player(game, B);
                game.enter_level(Dim::Nether).unwrap();
                prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
                game.set_state_named(HIT, "minecraft:gold_block", &[])
                    .unwrap();
                game.enter_level(Dim::Overworld).unwrap();
                prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
                game.set_state_named(HIT, "minecraft:stone", &[]).unwrap();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(A, |guest| {
                        guest.inventory[0] = Some(ItemStack::new("minecraft:dirt", 2));
                    })
                    .unwrap();

                let first = out().len();
                use_on(game, [15, 64, 4], 1);
                assert!(events.lock().unwrap().is_empty());
                let packets = out();
                packet_index(&packets[first..], |p| receipt(p, 1) == Some([false, false]));
                assert!(
                    !packets[first..]
                        .iter()
                        .any(|p| matches!(p, C::BlockUpdate(_)))
                );
                wire_ack(game, &out, 1);

                let size = game.common.border.size();
                game.common.border.set_size(2.0).unwrap();
                let start = out().len();
                use_on(game, HIT, 2);
                assert!(events.lock().unwrap().is_empty());
                correction_order(&out()[start..], 2, [false, false], false);
                game.common.border.set_size(size).unwrap();
                wire_ack(game, &out, 2);

                suppress.store(true, Ordering::Relaxed);
                let start = out().len();
                use_on(game, HIT, 3);
                correction_order(&out()[start..], 3, [true, true], true);
                let guest = game.server.lan.as_ref().unwrap().hub.guest(A).unwrap();
                assert_eq!(
                    guest.inventory[0],
                    Some(ItemStack::new("minecraft:dirt", 2))
                );
                assert_eq!(
                    *events.lock().unwrap(),
                    ["click", "nested", "restored", "block"]
                );
                assert_eq!(&*game.block_name(BESIDE), "minecraft:air");
                events.lock().unwrap().clear();
                suppress.store(false, Ordering::Relaxed);
                wire_ack(game, &out, 3);

                let start = out().len();
                let watcher_start = watcher().len();
                let _recording = Recording::start();
                let source = game.server_source();
                game.with_source(source, |game| {
                    assert!(matches!(
                        game.native_command_origin(0).unwrap(),
                        CommandSource::Console { .. }
                    ));
                    use_on(game, HIT, 4);
                    assert!(matches!(
                        game.native_command_origin(0).unwrap(),
                        CommandSource::Console { .. }
                    ));
                });
                assert!(game.server.command_source.is_none());
                assert!(game.server.lan_acting.is_none());
                assert_eq!(
                    *events.lock().unwrap(),
                    ["click", "nested", "restored", "block"]
                );
                assert_eq!(&*game.block_name(BESIDE), "minecraft:dirt");
                let host = game.server.lan.as_ref().unwrap();
                assert_eq!(
                    host.hub.guest(A).unwrap().inventory[0],
                    Some(ItemStack::new("minecraft:dirt", 4))
                );
                SEEN.with_borrow(|seen| {
                    let triggers: Vec<_> = seen
                        .iter()
                        .filter(|(kind, _)| {
                            matches!(
                                kind.as_str(),
                                "minecraft:placed_block" | "minecraft:any_block_use"
                            )
                        })
                        .map(|(kind, _)| kind.as_str())
                        .collect();
                    assert_eq!(
                        triggers,
                        ["minecraft:placed_block", "minecraft:any_block_use"]
                    );
                    let outer = seen
                        .iter()
                        .filter(|(kind, _)| kind == "minecraft:any_block_use")
                        .collect::<Vec<_>>();
                    assert_eq!(outer.len(), 1);
                    assert_eq!(outer[0].1, Some(ItemStack::new("minecraft:dirt", 1)));
                });

                let before = out();
                correction_order(&before[start..], 4, [true, false], true);
                let dirt = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:dirt")
                    .unwrap();
                packet_index(
                    &before[start..],
                    |p| matches!(p, C::BlockUpdate(p) if p.pos == BESIDE && p.state == dirt.0 as i32),
                );
                assert!(
                    !watcher()[watcher_start..]
                        .iter()
                        .any(|p| receipt(p, 4).is_some()
                            || matches!(p, C::BlockUpdate(p) if p.pos == HIT || p.pos == BESIDE))
                );
                wire_ack(game, &out, 4);
                assert!(
                    !watcher()
                        .iter()
                        .any(|p| (1..=4).any(|sequence| receipt(p, sequence).is_some())
                            || matches!(p, C::BlockChangedAck(_)))
                );

                game.as_guest(A, |game| {
                    game.as_guest(B, |game| {
                        let i = game.guest_index(A).unwrap();
                        let player = PlayerId::guest(
                            game.server.lan.as_ref().unwrap().hub.guest(A).unwrap(),
                        );
                        let level = LevelId::loaded(game, Dim::Overworld).unwrap();
                        let actor = ActorInLevel { player, level };
                        assert_eq!(
                            actor.with_guest(game, i, A, |_| ()),
                            Err(TurnRefused::Nested)
                        );
                    })
                    .unwrap();
                    assert_eq!(game.server.lan_acting, Some(A));
                })
                .unwrap();

                game.set_state_named(BESIDE, "minecraft:air", &[]).unwrap();
                let index = game.guest_index(A).unwrap();
                let guest = game.server.lan.as_ref().unwrap().hub.guest(A).unwrap();
                let actor = PlayerId::guest(guest);
                assert!(actor.view_at(game, index).is_some());
                disconnect.store(true, Ordering::Relaxed);
                game.server.audio_capture_active = true;
                game.server.pending_sounds.clear();
                RESUMES.with_borrow_mut(Vec::clear);
                COMMAND_ACTORS.with_borrow_mut(Vec::clear);
                use_on(game, HIT, 5);
                assert!(SOUND_PRESENT.get());
                assert!(game.server.pending_sounds.is_empty());
                let b = PlayerId::guest(game.server.lan.as_ref().unwrap().hub.guest(B).unwrap());
                assert!(COMMAND_ACTORS.with_borrow(|actors| actors.contains(&Some(b))));
                assert!(RESUMES.with_borrow(|resumes| {
                    resumes
                        .iter()
                        .any(|(id, valid)| id.player == actor && !valid)
                }));
                assert!(actor.view_at(game, index).is_none());
                assert_eq!(&*game.block_name(BESIDE), "minecraft:dirt");

                game.native_level_opened(Dim::Overworld);
                let old = LevelId::loaded(game, Dim::Overworld).unwrap();
                game.server.levels.0.clear();
                assert!(old.view(game).is_none());
                let reset = LevelId::loaded(game, Dim::Overworld).unwrap();
                game.native_level_closed(Dim::Overworld);
                assert!(reset.view(game).is_none());
                assert!(LevelId::loaded(game, Dim::Overworld).is_none());
                game.native_level_opened(Dim::Overworld);
                assert!(reset.view(game).is_none());
                let opened = LevelId::loaded(game, Dim::Overworld).unwrap();
                game.native_level_closed(Dim::Overworld);
                game.server_lifecycle_open_at(WorldEpoch::new(123));
                let lifecycle = LevelId::loaded(game, Dim::Overworld).unwrap();
                assert_ne!(opened, lifecycle);
                game.native_level_closed(Dim::Overworld);
                game.server
                    .levels
                    .0
                    .reset_active_closed_for_test(Dim::Overworld);
                assert_ne!(LevelId::loaded(game, Dim::Overworld).unwrap(), lifecycle);
            });
        });
    });
}
