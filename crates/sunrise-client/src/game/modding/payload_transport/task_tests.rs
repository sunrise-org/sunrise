//! Configuration admission through the serving Game and its main-thread payload executor.
use mod_api::{
    ConfigurationTask, ConfigurationTaskContext, ConfigurationTaskError, ConfigurationTaskRef,
    ConfigurationTaskRegistrationError, Hooks, IPayloadContext, IPayloadHandler, Identifier,
    ModEntry, PayloadBytes, PayloadId, Registrar, ServerHost,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
const CHANNEL: &str = "task_fixture:value";
const PEER_NAMES: [&str; 2] = ["ConfigPeerA", "ConfigPeerB"];
fn kind(name: &str) -> Identifier {
    Identifier::parse(&format!("task_fixture:{name}")).unwrap()
}
#[derive(Clone, Copy)]
enum Case {
    Reply,
    Stale,
    Failure,
    Vanilla,
    Tick,
    Panic,
    Refusal,
    ConcurrentPeers,
}
struct Fixture {
    case: Case,
    instances: Mutex<[Option<ConfigurationTaskRef>; 2]>,
    starts: Mutex<Vec<Identifier>>,
    joins: Mutex<Vec<String>>,
    type_reads: AtomicUsize,
    panic_staged_payload: AtomicBool,
}
struct Mod(Arc<Fixture>);
struct Reply(Arc<Fixture>);
impl IPayloadHandler for Reply {
    fn handle(
        &self,
        bytes: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        assert_eq!(std::thread::current().name(), Some("Server thread"));
        let [first, second] = *self.0.instances.lock().unwrap();
        let first = first.unwrap();
        let reply = match bytes.0.as_slice() {
            [step @ (7 | 8)] => {
                assert_eq!(*self.0.starts.lock().unwrap(), [kind("first")]);
                // Open a child explicitly: the scoped reply context needs its own mutable
                // borrow to finish, so it cannot be used inside the host's closure guard.
                let child = context.server_host().unwrap().transaction_open();
                context.finish_configuration_task(first).unwrap();
                context
                    .server_host()
                    .unwrap()
                    .transaction_close(&child, *step == 8);
                *step + 1
            }
            [10] => {
                assert_eq!(
                    *self.0.starts.lock().unwrap(),
                    [kind("first"), kind("second")]
                );
                assert_eq!(
                    context.finish_configuration_task(first),
                    Err(ConfigurationTaskError::StaleTask)
                );
                11
            }
            [11] => {
                context.finish_configuration_task(second.unwrap()).unwrap();
                12
            }
            [13] => {
                // B replies to its challenge with the instance saved for A instead.
                assert_ne!(first, second.unwrap());
                assert_eq!(
                    context.finish_configuration_task(first),
                    Err(ConfigurationTaskError::WrongConnection)
                );
                14
            }
            [step @ (15 | 16)] => {
                let instance = if *step == 15 { first } else { second.unwrap() };
                context.finish_configuration_task(instance).unwrap();
                17
            }
            other => panic!("unexpected configuration request: {other:?}"),
        };
        context
            .reply(&PayloadId(CHANNEL.into()), PayloadBytes(vec![reply]))
            .unwrap();
        Ok(())
    }
}
#[sunrise_mod_api_macros::__subscriptions]
impl Hooks for Mod {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .optional()
            .executes_on(mod_api::HandlerThread::Main)
            .configuration_bidirectional(
                PayloadId(CHANNEL.into()),
                Arc::new(Reply(self.0.clone())),
                Some(Arc::new(Reply(self.0.clone()))),
            );
    }
    fn register_configuration_tasks(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::RegisterConfigurationTasksEvent<'_>,
    ) {
        let task = |id, slot| Task {
            fixture: self.0.clone(),
            id,
            slot,
            waiting: false,
        };
        if matches!(self.0.case, Case::ConcurrentPeers) {
            if let Some(slot) = PEER_NAMES.iter().position(|&name| name == event.name) {
                event.register(task(kind("first"), slot)).unwrap();
            }
            return;
        }
        if event.name != "ConfigPeer" {
            return;
        }
        if matches!(self.0.case, Case::Vanilla) {
            assert!(
                !event
                    .connection
                    .has_channel(host, &PayloadId(CHANNEL.into()))
            );
            return;
        }
        event.register(task(kind("first"), 0)).unwrap();
        event.register(task(kind("second"), 1)).unwrap();
        if matches!(self.0.case, Case::Refusal) {
            let duplicate = event.register(task(kind("first"), 0));
            assert_eq!(
                duplicate,
                Err(ConfigurationTaskRegistrationError::DuplicateType(kind(
                    "first"
                )))
            );
            let empty =
                event.register(task(Identifier::from_parts("task_fixture", "").unwrap(), 0));
            assert_eq!(empty, Err(ConfigurationTaskRegistrationError::EmptyType));
            // Do not fail the connection explicitly: even ignored refusals abort admission.
        }
    }
    fn player_logged_in(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::PlayerLoggedInEvent<'_>,
    ) {
        if PEER_NAMES.contains(&event.name) {
            self.0.joins.lock().unwrap().push(event.name.into());
        }
    }
}

struct Task {
    fixture: Arc<Fixture>,
    id: Identifier,
    slot: usize,
    waiting: bool,
}
impl ConfigurationTask for Task {
    fn task_type(&self) -> &Identifier {
        self.fixture.type_reads.fetch_add(1, Ordering::Relaxed);
        &self.id
    }
    fn start(&mut self, host: &mut dyn ServerHost, context: &mut ConfigurationTaskContext<'_>) {
        assert_eq!(std::thread::current().name(), Some("Server thread"));
        self.fixture.starts.lock().unwrap().push(self.id.clone());
        self.fixture.instances.lock().unwrap()[self.slot] = Some(context.instance());
        match (self.fixture.case, self.slot) {
            (Case::Reply, 0) | (Case::Stale, 1) | (Case::ConcurrentPeers, _) => {
                let channel = PayloadId(CHANNEL.into());
                assert!(context.has_channel(host, &channel));
                let challenge = match self.fixture.case {
                    Case::Reply => 7,
                    Case::Stale => 10,
                    // Give the two connections distinct reply selectors even though the task type is the same.
                    _ => 15 + self.slot as u8,
                };
                context
                    .send_payload(host, &channel, PayloadBytes(vec![challenge]))
                    .unwrap();
            }
            (Case::Failure, 0) => {
                context.finish();
                context.fail("fixture failure");
                context.finish(); // Cannot overwrite failure.
            }
            (Case::Tick, 0) => self.waiting = true,
            (Case::Panic, 0) => {
                context.finish();
                context
                    .send_payload(host, &PayloadId(CHANNEL.into()), PayloadBytes(vec![99]))
                    .unwrap();
                self.fixture
                    .panic_staged_payload
                    .store(true, Ordering::Relaxed);
                panic!("fixture panic after staging completion and a payload");
            }
            _ => context.finish(),
        }
    }
    fn tick(&mut self, host: &mut dyn ServerHost, context: &mut ConfigurationTaskContext<'_>) {
        assert_eq!(std::thread::current().name(), Some("Server thread"));
        if self.waiting {
            self.waiting = false;
            let _abandoned = host.transaction_open();
            context.finish();
            context.finish();
        }
    }
}

fn vanilla_peer(
    port: u16,
) -> (
    sunrise_protocol::connection::Connection,
    sunrise_protocol::packets::configuration::SelectKnownPacks,
) {
    use sunrise_protocol::{
        connection::Connection,
        packets::{Empty, configuration as cfg, handshake, login},
    };
    let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(10)).unwrap();
    c.reader
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: sunrise_protocol::PROTOCOL_VERSION,
            host: "127.0.0.1".into(),
            port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .unwrap();
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: "ConfigPeer".into(),
            uuid: sunrise_protocol::crypto::offline_uuid("ConfigPeer"),
        })
        .encode(),
    )
    .unwrap();
    loop {
        let (id, body) = c.recv().unwrap();
        match login::Clientbound::decode(id, &body).unwrap() {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => break,
            p => panic!("unexpected login packet {p:?}"),
        }
    }
    c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
        .unwrap();

    loop {
        let (id, body) = c.recv().unwrap();
        if let cfg::Clientbound::SelectKnownPacks(known) =
            cfg::Clientbound::decode(id, &body).unwrap()
        {
            return (c, known);
        }
    }
}

fn publish_task_server(game: &mut super::super::Game<'_>) -> u16 {
    game.settings.render_distance = 2;
    game.settings.simulation_distance = 2;
    let options = sunrise_client::title::WorldOptions {
        world_type: sunrise_client::save::WorldPreset::Flat,
        ..Default::default()
    };
    game.open_integrated(
        "config_tasks",
        Some("Configuration tasks"),
        Some(1),
        true,
        Some(&options),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !game
        .remote
        .as_ref()
        .is_some_and(|r| r.logged_in && r.joined)
    {
        game.poll_remote().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    game.integrated
        .as_ref()
        .unwrap()
        .call(|server| {
            server.publish(super::super::lan::LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            })
        })
        .unwrap()
        .unwrap()
}

impl Fixture {
    fn new(case: Case) -> Self {
        Self {
            case,
            instances: Mutex::new([None; 2]),
            starts: Mutex::new(Vec::new()),
            joins: Mutex::new(Vec::new()),
            type_reads: AtomicUsize::new(0),
            panic_staged_payload: AtomicBool::new(false),
        }
    }
}

#[test]
#[ignore = "daily"]
fn serving_game_drives_configuration_tasks() {
    const NAME: &str =
        "game::payload_transport::task_tests::serving_game_drives_configuration_tasks";
    super::tests::require_resources();
    if super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "SUNRISE_TASK_CHILD",
        &[
            "reply", "stale", "failure", "vanilla", "tick", "panic", "refusal",
        ],
    ) {
        return;
    }
    let case = match std::env::var("SUNRISE_TASK_CHILD").unwrap().as_str() {
        "reply" => Case::Reply,
        "stale" => Case::Stale,
        "failure" => Case::Failure,
        "vanilla" => Case::Vanilla,
        "tick" => Case::Tick,
        "panic" => Case::Panic,
        "refusal" => Case::Refusal,
        _ => unreachable!(),
    };
    let fixture = Arc::new(Fixture::new(case));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "task_fixture",
            Box::new(Mod(fixture.clone())),
        )]
        .into(),
        || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                let port = publish_task_server(game);
                use sunrise_protocol::packets::{Empty, configuration as cfg, play};
                let (mut c, known) = if matches!(case, Case::Vanilla) {
                    vanilla_peer(port)
                } else {
                    let (c, _, known) = super::config_tests::configuration_peer(port, "ConfigPeer");
                    (c, known)
                };
                c.send(&cfg::Serverbound::SelectKnownPacks(known).encode())
                    .unwrap();
                let mut replies = match case {
                    Case::Reply => &[7_u8, 8, 9][..],
                    Case::Stale => &[10, 11, 12],
                    _ => &[],
                }
                .iter()
                .copied();
                loop {
                    let (id, body) = c.recv().unwrap();
                    match cfg::Clientbound::decode(id, &body).unwrap() {
                        cfg::Clientbound::CustomPayload(p) if p.channel == CHANNEL => {
                            let expected = replies.next().expect("too many configuration replies");
                            assert_eq!(p.data, [expected]);
                            if matches!(expected, 7 | 8 | 10 | 11) {
                                c.send(&cfg::Serverbound::CustomPayload(p).encode())
                                    .unwrap();
                            }
                        }
                        cfg::Clientbound::KeepAlive(p) => {
                            c.send(&cfg::Serverbound::KeepAlive(p).encode()).unwrap();
                        }
                        cfg::Clientbound::Disconnect(reason) => {
                            let reason =
                                sunrise_protocol::text::plain(&reason.reason, &Default::default());
                            assert!(reason.contains("Configuration error"), "{reason}");
                            let starts = fixture.starts.lock().unwrap();
                            match case {
                                Case::Failure => assert_eq!(*starts, [kind("first")]),
                                Case::Panic => {
                                    assert!(fixture.panic_staged_payload.load(Ordering::Relaxed));
                                    assert!(
                                        reason.contains("Configuration task callback panicked"),
                                        "{reason}"
                                    );
                                    assert_eq!(*starts, [kind("first")]);
                                }
                                Case::Refusal => {
                                    assert!(starts.is_empty());
                                    assert!(
                                        reason.contains(
                                            "Duplicate configuration task type: task_fixture:first"
                                        ),
                                        "{reason}"
                                    );
                                }
                                _ => panic!("unexpected configuration disconnect: {reason}"),
                            }
                            break;
                        }
                        cfg::Clientbound::FinishConfiguration(_) => {
                            assert!(!matches!(case, Case::Failure | Case::Panic | Case::Refusal));
                            assert_eq!(
                                replies.next(),
                                None,
                                "FinishConfiguration overtook a reply"
                            );
                            let expected = if matches!(case, Case::Vanilla) {
                                vec![]
                            } else {
                                vec![kind("first"), kind("second")]
                            };
                            assert_eq!(*fixture.starts.lock().unwrap(), expected);
                            c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
                                .unwrap();
                            loop {
                                let (id, body) = c.recv().unwrap();
                                if matches!(
                                    play::Clientbound::decode(id, &body).unwrap(),
                                    play::Clientbound::Login(_)
                                ) {
                                    break;
                                }
                            }
                            break;
                        }
                        _ => {}
                    }
                }
                assert_eq!(
                    fixture.type_reads.load(Ordering::Relaxed),
                    match case {
                        Case::Vanilla => 0,
                        Case::Refusal => 4,
                        _ => 2,
                    }
                );
                game.leave_server(None).unwrap();
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn peer_completion_rejects_other_connections() {
    use sunrise_protocol::{
        connection::Connection,
        packets::{Empty, configuration as cfg, play},
    };
    super::tests::require_resources();
    if super::super::native_entity_refs::tests::run_fresh_cases(
        "game::payload_transport::task_tests::peer_completion_rejects_other_connections",
        "SUNRISE_TASK_PEER_CHILD",
        &["cross_peer"],
    ) {
        return;
    }
    let fixture = Arc::new(Fixture::new(Case::ConcurrentPeers));
    let mods = vec![ModEntry::new(
        "task_fixture",
        Box::new(Mod(fixture.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let port = publish_task_server(game);
            let [(mut a, _, packs_a), (mut b, _, packs_b)] =
                PEER_NAMES.map(|name| super::config_tests::configuration_peer(port, name));
            a.send(&cfg::Serverbound::SelectKnownPacks(packs_a).encode())
                .unwrap();
            b.send(&cfg::Serverbound::SelectKnownPacks(packs_b).encode())
                .unwrap();

            let next = |c: &mut Connection| loop {
                let (id, body) = c.recv().unwrap();
                match cfg::Clientbound::decode(id, &body).unwrap() {
                    cfg::Clientbound::CustomPayload(p) if p.channel == CHANNEL => {
                        break Some(p.data);
                    }
                    cfg::Clientbound::KeepAlive(p) => {
                        c.send(&cfg::Serverbound::KeepAlive(p).encode()).unwrap();
                    }
                    cfg::Clientbound::FinishConfiguration(_) => break None,
                    cfg::Clientbound::Disconnect(p) => {
                        panic!("disconnected in configuration: {p:?}")
                    }
                    _ => {}
                }
            };
            let reply = |c: &mut Connection, step| {
                c.send(
                    &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: CHANNEL.into(),
                        data: vec![step],
                    })
                    .encode(),
                )
                .unwrap();
            };

            // Both challenges are outstanding, each from the first task of the same type.
            assert_eq!(next(&mut a), Some(vec![15]));
            assert_eq!(next(&mut b), Some(vec![16]));
            assert_eq!(
                *fixture.starts.lock().unwrap(),
                [kind("first"), kind("first")]
            );
            reply(&mut b, 13);
            assert_eq!(
                next(&mut b),
                Some(vec![14]),
                "B did not report WrongConnection"
            );

            // A premature advance of either connection would put FinishConfiguration
            // before its own reply, or make the handler's own-instance completion fail.
            for (peer, step) in [(&mut a, 15), (&mut b, 16)] {
                reply(peer, step);
                assert_eq!(next(peer), Some(vec![17]), "own task must still be running");
                assert_eq!(next(peer), None, "own task must finish configuration");
                peer.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
                    .unwrap();
                loop {
                    let (id, body) = peer.recv().unwrap();
                    match play::Clientbound::decode(id, &body).unwrap() {
                        play::Clientbound::Login(_) => break,
                        play::Clientbound::Disconnect(p) => {
                            panic!("disconnected before play login: {p:?}")
                        }
                        _ => {}
                    }
                }
            }
            // The play Login is sent in the same tick as the join announcement.
            // Read the hook's record between ticks, while both connections are still open.
            let joins = game
                .integrated
                .as_ref()
                .unwrap()
                .call(move |_| fixture.joins.lock().unwrap().clone())
                .unwrap();
            assert_eq!(joins, PEER_NAMES.map(str::to_owned));
            game.leave_server(None).unwrap();
        });
    });
}
