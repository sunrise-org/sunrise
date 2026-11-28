//! Configuration-phase sync of mod-declared `sync(true)` static custom registries over a
//! real loopback connection (`Lan` + `Hub` + `net::Session`), matching NeoForge's
//! `SyncRegistries`, `RegistryManager`, `ConfigurationInitialization` and `ClientPayloadHandler`.
extern crate sunrise_client_lib as sunrise_client;
use mod_api::Identifier;
use std::{
    any::Any,
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use sunrise_client::{
    chunk::MIN_Y,
    custom_registry_net::{
        self, LocalRegistries, Snapshot, SnapshotRegistry, SyncedCustomRegistries,
    },
    lan::{
        Lan, NetConfig,
        chunks::ChunkEncoder,
        data::SyncData,
        hub::{HostEvent, Hub, HubConfig, TickCtx},
    },
    net::{Event, Session},
    world::World,
};
use sunrise_protocol::{
    PROTOCOL_VERSION,
    connection::Connection,
    crypto::offline_uuid,
    packets::{
        Empty,
        configuration::{self as cfg, ClientInformation, RegistryTags, TagEntry},
        handshake, login,
    },
};
use sunrise_world::{
    item_defaults::ItemData,
    native_load_context::{DataLoadOrigin, LoadContext, LoadInputs},
    registry::BlockStateId,
};

struct LocalDir {
    id: String,
    tag_dir: String,
    sync: bool,
    entries: Vec<Identifier>,
}

impl LocalDir {
    fn arc(
        id: &str,
        sync: bool,
        entries: &[&str],
    ) -> Arc<dyn mod_api::__private::RegistryDirectory> {
        let parsed = Identifier::parse_namespaced(id).unwrap();
        Arc::new(Self {
            id: id.to_owned(),
            tag_dir: format!("{}/{}", parsed.namespace(), parsed.path()),
            sync,
            entries: entries
                .iter()
                .map(|entry| Identifier::parse_namespaced(entry).unwrap())
                .collect(),
        })
    }
}

impl mod_api::__private::RegistryDirectory for LocalDir {
    fn id(&self) -> &str {
        &self.id
    }
    fn sync(&self) -> bool {
        self.sync
    }
    fn entry_ids(&self) -> &[Identifier] {
        &self.entries
    }
    fn contains_id(&self, id: &str) -> bool {
        self.entries.iter().any(|entry| entry.to_string() == id)
    }
    fn tag_directory(&self) -> &str {
        &self.tag_dir
    }
    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }
}

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
            "sunrise-custom-registry-join-{name}-{}",
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
    fn start(sync: SyncData) -> Self {
        let sync = Arc::new(sync);
        let net = NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "Custom registry join".into(), 8)
        };
        let lan = Lan::start_with(sync.clone(), net).unwrap();
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
            }
        }
    }
}

fn connect(
    port: u16,
    name: &str,
    data: &Arc<ItemData>,
    local_registries: LocalRegistries,
) -> Session {
    Session::connect(
        &format!("127.0.0.1:{port}"),
        name,
        None,
        ClientInformation::new(8),
        Arc::new(Default::default()),
        Arc::clone(data),
        Arc::default(),
        local_registries,
    )
}

fn drive_join(
    host: &mut Host,
    session: &Session,
) -> Result<
    (
        Arc<sunrise_protocol::registry::Dynamic>,
        SyncedCustomRegistries,
    ),
    String,
> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        host.tick();
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
                Event::Joined {
                    registries,
                    custom_registries,
                    ..
                } => return Ok((registries, custom_registries)),
                Event::Disconnected { reason, .. } => return Err(reason),
                _ => {}
            }
        }
        assert!(Instant::now() < deadline, "timed out waiting for join");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Matching synced registries join cleanly across differing registration orders,
/// extra client-only entries/registries and empty absent server registries are accepted,
/// and `UpdateTags` decodes custom registry tags via the server's raw ID order.
#[test]
#[ignore = "daily"]
fn matching_synced_registries_join_and_decode_tags_across_differing_orders() {
    let _publication = custom_registry_net::isolate_server_publication();
    custom_registry_net::serve(Some(Arc::new(Snapshot {
        registries: vec![
            SnapshotRegistry {
                registry_id: "chemlib:gas".into(),
                entries: vec!["boiler:steam".into(), "chemlib:oxygen".into()],
            },
            SnapshotRegistry {
                registry_id: "chemlib:empty".into(),
                entries: vec![],
            },
        ],
    })));
    let local = LocalRegistries::new(vec![
        // Client registered oxygen (0) before steam (1), plus an extra client-only entry.
        LocalDir::arc(
            "chemlib:gas",
            true,
            &["chemlib:oxygen", "boiler:steam", "clientonly:neon"],
        ),
        LocalDir::arc("clientonly:extra", true, &["clientonly:a"]),
    ]);

    let mut sync = SyncData::empty();
    // Server tag `boiler:hot` contains server raw_id 0 (`boiler:steam`).
    sync.custom_registry_tags.push(RegistryTags {
        registry: "chemlib:gas".into(),
        tags: vec![TagEntry {
            name: "boiler:hot".into(),
            ids: vec![0],
        }],
    });
    let (_resources, data) = Resources::new("matching");
    let mut host = Host::start(sync);
    let session = connect(host.port(), "Alchemist", &data, local);
    let (registries, synced) = drive_join(&mut host, &session).expect("matching client joins");

    // Server raw_id 0 decodes to `boiler:steam` despite client having oxygen at slot 0.
    let raw_ids = registries.tag("chemlib:gas", "boiler:hot").unwrap();
    let decoded: Vec<String> = raw_ids
        .iter()
        .map(|&raw_id| synced.entry_id("chemlib:gas", raw_id).unwrap().to_string())
        .collect();
    assert_eq!(decoded, vec!["boiler:steam"]);
}

/// Client missing a non-empty synced registry disconnects naming the registry
/// (`RegistryManager.applySnapshot:147-149`, `ClientPayloadHandler.java:114-118`).
#[test]
#[ignore = "daily"]
fn client_missing_non_empty_synced_registry_disconnects_naming_registry() {
    let _publication = custom_registry_net::isolate_server_publication();
    custom_registry_net::serve(Some(Arc::new(Snapshot {
        registries: vec![SnapshotRegistry {
            registry_id: "chemlib:gas".into(),
            entries: vec!["chemlib:oxygen".into()],
        }],
    })));
    let local = LocalRegistries::new(vec![]);

    let (_resources, data) = Resources::new("missing-reg");
    let mut host = Host::start(SyncData::empty());
    let session = connect(host.port(), "PlainSunrise", &data, local);
    let reason = drive_join(&mut host, &session).unwrap_err();
    assert!(
        reason.contains("chemlib:gas"),
        "expected disconnect naming chemlib:gas, got: {reason}"
    );
}

/// Client missing an entry sent by the server disconnects naming the registry and unknown entry
/// (`RegistryManager.applySnapshot:176-179`, `ClientPayloadHandler.java:104-110`).
#[test]
#[ignore = "daily"]
fn client_missing_synced_entry_disconnects_naming_registry_and_entry() {
    let _publication = custom_registry_net::isolate_server_publication();
    custom_registry_net::serve(Some(Arc::new(Snapshot {
        registries: vec![SnapshotRegistry {
            registry_id: "chemlib:gas".into(),
            entries: vec!["chemlib:oxygen".into(), "boiler:steam".into()],
        }],
    })));
    let local = LocalRegistries::new(vec![LocalDir::arc(
        "chemlib:gas",
        true,
        &["chemlib:oxygen"],
    )]);

    let (_resources, data) = Resources::new("missing-entry");
    let mut host = Host::start(SyncData::empty());
    let session = connect(host.port(), "HalfModded", &data, local);
    let reason = drive_join(&mut host, &session).unwrap_err();
    assert!(
        reason.contains("chemlib:gas") && reason.contains("boiler:steam"),
        "expected disconnect naming chemlib:gas and boiler:steam, got: {reason}"
    );
}

/// Vanilla client joining a server with `sync(true)` custom registries is never disconnected
/// and receives neither `sunrise:custom_registries` nor custom-registry `UpdateTags` groups
/// (`ConfigurationInitialization.configureEarlyTasks:38-42`, `NetworkInitialization.java:42-58`).
#[test]
#[ignore = "daily"]
fn vanilla_client_joins_without_custom_registry_payload_or_disconnect() {
    let _publication = custom_registry_net::isolate_server_publication();
    custom_registry_net::serve(Some(Arc::new(Snapshot {
        registries: vec![SnapshotRegistry {
            registry_id: "chemlib:gas".into(),
            entries: vec!["chemlib:oxygen".into()],
        }],
    })));
    let mut sync = SyncData::empty();
    sync.custom_registry_tags.push(RegistryTags {
        registry: "chemlib:gas".into(),
        tags: vec![TagEntry {
            name: "boiler:hot".into(),
            ids: vec![0],
        }],
    });
    let net = NetConfig {
        announce: false,
        ..NetConfig::loopback(0, "Vanilla custom registry join".into(), 8)
    };
    let lan = Lan::start_with(Arc::new(sync), net).unwrap();
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
    loop {
        let (id, payload) = c.recv().unwrap();
        match cfg::Clientbound::decode(id, &payload).unwrap() {
            cfg::Clientbound::SelectKnownPacks(_) => c
                .send(
                    &cfg::Serverbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: vec![] })
                        .encode(),
                )
                .unwrap(),
            cfg::Clientbound::CustomPayload(p) if p.channel == custom_registry_net::CHANNEL => {
                panic!("vanilla client must not receive sunrise:custom_registries");
            }
            cfg::Clientbound::UpdateTags(tags) => {
                assert!(
                    !tags.registries.iter().any(|r| r.registry == "chemlib:gas"),
                    "vanilla client must not receive custom registry tags in UpdateTags"
                );
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
