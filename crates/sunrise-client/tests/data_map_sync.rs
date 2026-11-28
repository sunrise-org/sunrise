//! Configuration negotiation and play-phase synchronization of synced data maps over a real
//! loopback connection (`Lan` + `Hub` + `net::Session`), matching NeoForge's
//! `RegistryDataMapNegotiation`, `ClientRegistryManager`, and `RegistryDataMapSyncPayload`.
extern crate sunrise_client_lib as sunrise_client;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::{Duration, Instant},
};
use sunrise_client::{
    chunk::MIN_Y,
    data_map_net::{self, KnownEntry, LocalDataMaps, SyncedDataMapsSnapshot},
    lan::{
        Lan, NetConfig,
        chunks::ChunkEncoder,
        data::SyncData,
        hub::{HostEvent, Hub, HubConfig, TickCtx},
    },
    net::{Event, LocalInventory, Session},
    world::World,
};
use sunrise_core::nbt::Tag;
use sunrise_protocol::{
    PROTOCOL_VERSION,
    connection::Connection,
    crypto::offline_uuid,
    packets::{
        Empty,
        configuration::{self as cfg, ClientInformation},
        handshake, login, play,
    },
};
use sunrise_world::{
    item_defaults::ItemData,
    native_load_context::{DataLoadOrigin, LoadContext, LoadInputs},
    registry::BlockStateId,
};

fn load_context(origin: DataLoadOrigin) -> LoadContext {
    LoadContext::new(
        origin,
        Arc::new(LoadInputs::new(BTreeMap::new(), Default::default())),
        [],
    )
}

struct Resources(std::path::PathBuf);

impl Resources {
    fn new(name: &str) -> (Self, Arc<ItemData>) {
        let root = std::env::temp_dir().join(format!(
            "sunrise-data-map-sync-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let data = ItemData::load(&root, load_context(DataLoadOrigin::ClientPreload)).unwrap();
        (Self(root), Arc::new(data))
    }
}

impl Drop for Resources {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

struct Host {
    hub: Hub,
    world: World,
    next_entity: u32,
    joined: Vec<u32>,
}

impl Host {
    fn start(sync: SyncData, server_known: LocalDataMaps) -> Self {
        let sync = Arc::new(sync);
        let net = NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "Data map sync".into(), 8)
        };
        let lan = Lan::start_with_data_maps(sync.clone(), net, server_known).unwrap();
        let encoder = ChunkEncoder::new(
            [] as [(Arc<str>, bool); 0],
            &sunrise_protocol::registry::Builtin::default(),
            &[],
            &[],
            Default::default(),
        );
        let config = HubConfig {
            allow_flight: false,
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 3,
            simulation_distance: 3,
            integrated: true,
            dimension: "minecraft:overworld".into(),
            seed: 1,
            max_players: 8,
            levels: Default::default(),
            level_seeds: Default::default(),
        };
        let load = Arc::new(
            load_context(DataLoadOrigin::ServerDataLoad)
                .loop_context(sunrise_world::native_load_context::LoopSide::Server, []),
        );
        Self {
            hub: Hub::new(lan, sync, encoder, config, load),
            world: World::new(BlockStateId(0), 0),
            next_entity: 100,
            joined: Vec::new(),
        }
    }

    fn port(&self) -> u16 {
        self.hub.lan.port
    }

    fn tick(&mut self) {
        let mut ctx = TickCtx {
            sea_level: 63,
            world: &mut self.world,
            entities: &[],
            next_entity: &mut self.next_entity,
            spawn: [8.5, f64::from(MIN_Y + 5), 8.5],
            spawn_angle: [0.0; 2],
            spawn_dimension: "minecraft:overworld",
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &sunrise_client::worldborder::WorldBorder::default(),
            streaming: None,
        };
        for event in self.hub.tick(&mut ctx) {
            if let HostEvent::Joined(conn, _) = event {
                self.joined.push(conn);
                if let Some(i) = self.hub.begin_player_join(conn, &ctx) {
                    self.hub
                        .send_commands(i, self.hub.commands_packet(self.hub.guests[i].permission));
                    self.hub.finish_player_join(i, &ctx);
                }
            }
        }
    }
}

fn connect(port: u16, name: &str, data: &Arc<ItemData>, local_data_maps: LocalDataMaps) -> Session {
    Session::connect(
        &format!("127.0.0.1:{port}"),
        name,
        None,
        ClientInformation::new(8),
        Arc::new(Default::default()),
        Arc::clone(data),
        Arc::default(),
        LocalInventory {
            data_maps: local_data_maps,
            ..Default::default()
        },
    )
}

type SyncPayload = (String, HashMap<String, Vec<(String, Tag)>>);

fn drive_join(host: &mut Host, session: &Session) -> Result<Vec<SyncPayload>, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut pending = Vec::new();
    loop {
        host.tick();
        let mut joined = false;
        for receipt in session.poll() {
            let (event, _credit) = receipt.into_parts();
            let event = match event {
                Event::Configuration(task) => {
                    let Some(effect) = task.handle() else {
                        continue;
                    };
                    effect
                }
                Event::Cookie(task) => {
                    task.handle();
                    continue;
                }
                Event::FinishConfiguration(change) => {
                    change.acknowledge();
                    continue;
                }
                event => event,
            };
            match event {
                Event::Joined { .. } => joined = true,
                Event::Disconnected { reason, .. } => return Err(reason),
                Event::Packet(packet) => {
                    if let play::Clientbound::CustomPayload(p) = *packet
                        && p.channel == data_map_net::SYNC_CHANNEL
                    {
                        pending.push(data_map_net::decode_sync_payload(&p.data).unwrap());
                    }
                }
                _ => {}
            }
        }
        if joined {
            return Ok(pending);
        }
        assert!(Instant::now() < deadline, "timed out waiting for join");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_data_map_sync(
    host: &mut Host,
    session: &Session,
    pending: &mut Vec<SyncPayload>,
    expected_registry: &str,
) -> HashMap<String, Vec<(String, Tag)>> {
    if let Some(pos) = pending.iter().position(|(reg, _)| reg == expected_registry) {
        return pending.remove(pos).1;
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        host.tick();
        for receipt in session.poll() {
            let (event, _credit) = receipt.into_parts();
            match event {
                Event::Disconnected { reason, .. } => {
                    panic!(
                        "disconnected while waiting for sunrise:data_map_sync for {expected_registry}: {reason}"
                    );
                }
                Event::Packet(packet) => {
                    if let play::Clientbound::CustomPayload(p) = *packet
                        && p.channel == data_map_net::SYNC_CHANNEL
                    {
                        let (registry, maps) = data_map_net::decode_sync_payload(&p.data).unwrap();
                        if registry == expected_registry {
                            return maps;
                        }
                        pending.push((registry, maps));
                    }
                }
                _ => {}
            }
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for sunrise:data_map_sync for {expected_registry}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn known(registry: &str, map_id: &str, mandatory: bool) -> KnownEntry {
    KnownEntry {
        registry: registry.to_owned(),
        map_id: map_id.to_owned(),
        mandatory,
    }
}

/// Optional and Required maps sync on join and update on reload (removing dropped entries).
#[test]
#[ignore = "daily"]
fn optional_and_required_maps_sync_on_join_and_update_on_reload() {
    let _publication = data_map_net::isolate_server_publication();
    let entries = LocalDataMaps::from_entries(vec![
        known("minecraft:item", "chemlib:fuel", true),
        known("minecraft:item", "chemlib:hint", false),
    ]);

    let mut initial = BTreeMap::new();
    initial.insert(
        "minecraft:item".to_owned(),
        vec![
            (
                "chemlib:fuel".to_owned(),
                vec![
                    ("minecraft:coal".to_owned(), Tag::Int(1600)),
                    ("minecraft:stick".to_owned(), Tag::Int(100)),
                ],
            ),
            (
                "chemlib:hint".to_owned(),
                vec![("minecraft:coal".to_owned(), Tag::Int(1))],
            ),
        ],
    );
    data_map_net::serve(Some(Arc::new(SyncedDataMapsSnapshot {
        registries: initial,
    })));

    let (_resources, data) = Resources::new("join-and-reload");
    let mut host = Host::start(SyncData::empty(), entries.clone());
    let session = connect(host.port(), "SyncClient", &data, entries);
    let mut pending = drive_join(&mut host, &session).expect("matching Sunrise client must join");

    // Wait for initial play-phase sync payload.
    let synced = wait_data_map_sync(&mut host, &session, &mut pending, "minecraft:item");
    assert_eq!(
        synced["chemlib:fuel"],
        vec![
            ("minecraft:coal".to_owned(), Tag::Int(1600)),
            ("minecraft:stick".to_owned(), Tag::Int(100)),
        ]
    );
    assert_eq!(
        synced["chemlib:hint"],
        vec![("minecraft:coal".to_owned(), Tag::Int(1))]
    );

    // Simulate /reload: update coal and remove stick completely.
    let mut reloaded = BTreeMap::new();
    reloaded.insert(
        "minecraft:item".to_owned(),
        vec![
            (
                "chemlib:fuel".to_owned(),
                vec![("minecraft:coal".to_owned(), Tag::Int(2400))],
            ),
            ("chemlib:hint".to_owned(), vec![]),
        ],
    );
    data_map_net::serve(Some(Arc::new(SyncedDataMapsSnapshot {
        registries: reloaded,
    })));
    for guest in host.hub.online_players() {
        data_map_net::sync_connection(&guest.sender, &host.hub.lan.shared.data_maps_known);
    }

    let updated = wait_data_map_sync(&mut host, &session, &mut pending, "minecraft:item");
    assert_eq!(
        updated["chemlib:fuel"],
        vec![("minecraft:coal".to_owned(), Tag::Int(2400))]
    );
    assert!(updated["chemlib:hint"].is_empty());
}

/// Client missing a `DataMapSync::Required` map disconnects naming the map
/// (`ClientRegistryManager.handleKnownDataMaps`).
#[test]
#[ignore = "daily"]
fn required_map_missing_on_client_disconnects_naming_map() {
    let _publication = data_map_net::isolate_server_publication();
    let server_known =
        LocalDataMaps::from_entries(vec![known("minecraft:item", "chemlib:fuel", true)]);
    let client_known = LocalDataMaps::from_entries(vec![]);

    let (_resources, data) = Resources::new("missing-required");
    let mut host = Host::start(SyncData::empty(), server_known);
    let session = connect(host.port(), "MissingRequired", &data, client_known);
    let reason = drive_join(&mut host, &session).unwrap_err();
    assert!(
        reason.contains("chemlib:fuel") && reason.contains("minecraft:item"),
        "expected disconnect naming chemlib:fuel (minecraft:item), got: {reason}"
    );
}

/// Sunrise client requiring a `DataMapSync::Required` map absent on an optional-only server
/// disconnects naming the map (`ClientRegistryManager.handleKnownDataMaps`).
#[test]
#[ignore = "daily"]
fn client_required_map_missing_on_optional_server_disconnects_naming_map() {
    let _publication = data_map_net::isolate_server_publication();
    let server_known =
        LocalDataMaps::from_entries(vec![known("minecraft:item", "chemlib:opt_hint", false)]);
    let client_known =
        LocalDataMaps::from_entries(vec![known("minecraft:item", "boiler:heat", true)]);

    let (_resources, data) = Resources::new("client-required-missing-on-server");
    let mut host = Host::start(SyncData::empty(), server_known);
    let session = connect(host.port(), "ClientNeedsHeat", &data, client_known);
    let reason = drive_join(&mut host, &session).unwrap_err();
    assert!(
        reason.contains("boiler:heat") && reason.contains("minecraft:item"),
        "expected disconnect naming boiler:heat (minecraft:item), got: {reason}"
    );
}

/// Vanilla client joining a server with a `DataMapSync::Required` map is disconnected
/// with a plain message naming the map (`RegistryDataMapNegotiation.run:37-50`).
#[test]
#[ignore = "daily"]
fn required_map_disconnects_vanilla_client() {
    let _publication = data_map_net::isolate_server_publication();
    let server_known =
        LocalDataMaps::from_entries(vec![known("minecraft:item", "chemlib:fuel", true)]);

    let net = NetConfig {
        announce: false,
        ..NetConfig::loopback(0, "Vanilla required data map".into(), 8)
    };
    let lan = Lan::start_with_data_maps(Arc::new(SyncData::empty()), net, server_known).unwrap();
    let mut c = Connection::connect("127.0.0.1", lan.port, Duration::from_secs(5)).unwrap();
    c.reader
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: "127.0.0.1".into(),
            port: lan.port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .unwrap();
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: "Vanilla".into(),
            uuid: offline_uuid("Vanilla"),
        })
        .encode(),
    )
    .unwrap();
    loop {
        let (id, payload) = c.recv().unwrap();
        match login::Clientbound::decode(id, &payload).unwrap() {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => {
                c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
                    .unwrap();
                break;
            }
            other => panic!("unexpected login packet {other:?}"),
        }
    }
    c.send(&cfg::Serverbound::ClientInformation(ClientInformation::new(4)).encode())
        .unwrap();
    let mut name = ::sunrise_core::vanilla_bytes::Writer::new();
    name.string("vanilla");
    c.send(
        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: "minecraft:brand".into(),
            data: name.data,
        })
        .encode(),
    )
    .unwrap();
    let reason = loop {
        let (id, payload) = c.recv().unwrap();
        match cfg::Clientbound::decode(id, &payload).unwrap() {
            cfg::Clientbound::SelectKnownPacks(_) => c
                .send(
                    &cfg::Serverbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: vec![] })
                        .encode(),
                )
                .unwrap(),
            cfg::Clientbound::Disconnect(d) => {
                break sunrise_protocol::text::plain(&d.reason, &Default::default());
            }
            cfg::Clientbound::FinishConfiguration(_) => {
                panic!("vanilla client must be disconnected when server has mandatory data maps");
            }
            _ => {}
        }
    };
    assert!(
        reason.contains("chemlib:fuel") && reason.contains("minecraft:item"),
        "expected disconnect naming chemlib:fuel (minecraft:item), got: {reason}"
    );
}

/// Optional-only server allows both a vanilla client and a Sunrise client without the map.
#[test]
#[ignore = "daily"]
fn optional_only_server_allows_vanilla_client_and_sunrise_client_without_map() {
    let _publication = data_map_net::isolate_server_publication();
    let server_known =
        LocalDataMaps::from_entries(vec![known("minecraft:item", "chemlib:opt_hint", false)]);
    let client_known = LocalDataMaps::from_entries(vec![]);

    // 1. Sunrise client without the optional map joins cleanly.
    let (_resources, data) = Resources::new("optional-only");
    let mut host = Host::start(SyncData::empty(), server_known);
    let session = connect(host.port(), "NoModSunrise", &data, client_known);
    drive_join(&mut host, &session).expect("Sunrise client without optional map must join cleanly");

    // 2. Vanilla client joins cleanly without receiving sunrise:data_map_known.
    let mut c = Connection::connect("127.0.0.1", host.port(), Duration::from_secs(5)).unwrap();
    c.reader
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: "127.0.0.1".into(),
            port: host.port(),
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .unwrap();
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: "VanillaOpt".into(),
            uuid: offline_uuid("VanillaOpt"),
        })
        .encode(),
    )
    .unwrap();
    loop {
        let (id, payload) = c.recv().unwrap();
        match login::Clientbound::decode(id, &payload).unwrap() {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => {
                c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
                    .unwrap();
                break;
            }
            other => panic!("unexpected login packet {other:?}"),
        }
    }
    c.send(&cfg::Serverbound::ClientInformation(ClientInformation::new(4)).encode())
        .unwrap();
    let mut name = ::sunrise_core::vanilla_bytes::Writer::new();
    name.string("vanilla");
    c.send(
        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: "minecraft:brand".into(),
            data: name.data,
        })
        .encode(),
    )
    .unwrap();
    loop {
        let (id, payload) = c.recv().unwrap();
        match cfg::Clientbound::decode(id, &payload).unwrap() {
            cfg::Clientbound::SelectKnownPacks(_) => c
                .send(
                    &cfg::Serverbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: vec![] })
                        .encode(),
                )
                .unwrap(),
            cfg::Clientbound::CustomPayload(p) if p.channel == data_map_net::KNOWN_CHANNEL => {
                panic!("vanilla client must not receive sunrise:data_map_known");
            }
            cfg::Clientbound::Disconnect(d) => {
                panic!("vanilla client must not be disconnected: {:?}", d.reason);
            }
            cfg::Clientbound::FinishConfiguration(_) => {
                c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
                    .unwrap();
                break;
            }
            _ => {}
        }
    }
}
