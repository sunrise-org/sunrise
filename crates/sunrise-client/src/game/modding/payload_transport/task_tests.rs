//! Configuration admission through the serving Game and its main-thread payload executor.
use mod_api::{
    ConfigurationConnection, ConfigurationTask, ConfigurationTaskError, Hooks, IPayloadContext,
    IPayloadHandler, Identifier, ModEntry, PayloadBytes, PayloadId, Registrar, ServerHost,
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
const CHANNEL: &str = "task_fixture:value";
fn kind(name: &str) -> Identifier {
    Identifier::parse(&format!("task_fixture:{name}")).unwrap()
}
#[derive(Clone, Copy)]
enum Case {
    Reply,
    WrongType,
    Failure,
    Vanilla,
    Tick,
}
struct Fixture {
    case: Case,
    connection: Mutex<Option<ConfigurationConnection>>,
    starts: Mutex<Vec<Identifier>>,
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
        assert_eq!(bytes.0, [7]);
        let connection = self.0.connection.lock().unwrap().unwrap();
        connection
            .finish(context.server_host().unwrap(), &kind("first"))
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
        if event.name != "ConfigPeer" {
            return;
        }
        *self.0.connection.lock().unwrap() = Some(event.connection);
        if matches!(self.0.case, Case::Vanilla) {
            assert!(
                !event
                    .connection
                    .has_channel(host, &PayloadId(CHANNEL.into()))
            );
            return;
        }
        event.tasks.push(ConfigurationTask::new(kind("first")));
        event.tasks.push(ConfigurationTask::new(kind("second")));
    }
    fn configuration_task_start(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::ConfigurationTaskStartEvent<'_>,
    ) {
        self.0.starts.lock().unwrap().push(event.task_type.clone());
        if event.task_type == &kind("second") {
            event.connection.finish(host, event.task_type).unwrap();
            return;
        }
        match self.0.case {
            Case::Reply => event
                .connection
                .send_payload(host, &PayloadId(CHANNEL.into()), PayloadBytes(vec![7]))
                .unwrap(),
            Case::WrongType => assert_eq!(
                event.connection.finish(host, &kind("wrong")),
                Err(ConfigurationTaskError::WrongType)
            ),
            Case::Failure => event.connection.fail(host, "fixture failure"),
            Case::Vanilla => {
                assert!(
                    !event
                        .connection
                        .has_channel(host, &PayloadId(CHANNEL.into()))
                );
                event.connection.finish(host, event.task_type).unwrap();
            }
            Case::Tick => {}
        }
    }
    fn configuration_task_tick(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::ConfigurationTaskTickEvent<'_>,
    ) {
        if matches!(self.0.case, Case::Tick) {
            event.finished = true;
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

#[test]
#[ignore = "daily"]
fn serving_game_drives_configuration_tasks() {
    const NAME: &str =
        "game::payload_transport::task_tests::serving_game_drives_configuration_tasks";
    super::tests::require_resources();
    if super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "SUNRISE_TASK_CHILD",
        &["reply", "wrong", "failure", "vanilla", "tick"],
    ) {
        return;
    }
    let case = match std::env::var("SUNRISE_TASK_CHILD").unwrap().as_str() {
        "reply" => Case::Reply,
        "wrong" => Case::WrongType,
        "failure" => Case::Failure,
        "vanilla" => Case::Vanilla,
        "tick" => Case::Tick,
        _ => unreachable!(),
    };
    let fixture = Arc::new(Fixture {
        case,
        connection: Mutex::new(None),
        starts: Mutex::new(Vec::new()),
    });
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "task_fixture",
            Box::new(Mod(fixture.clone())),
        )]
        .into(),
        || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
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
                let port = game
                    .integrated
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
                    .unwrap();
                use sunrise_protocol::packets::{Empty, configuration as cfg, play};
                let (mut c, known) = if matches!(case, Case::Vanilla) {
                    vanilla_peer(port)
                } else {
                    let (c, _, known) = super::config_tests::configuration_peer(port);
                    (c, known)
                };
                c.send(&cfg::Serverbound::SelectKnownPacks(known).encode())
                    .unwrap();
                let mut replies = 0;
                loop {
                    let (id, body) = c.recv().unwrap();
                    match cfg::Clientbound::decode(id, &body).unwrap() {
                        cfg::Clientbound::CustomPayload(p) if p.channel == CHANNEL => {
                            assert!(matches!(case, Case::Reply));
                            assert_eq!(p.data, [7]);
                            assert_eq!(*fixture.starts.lock().unwrap(), vec![kind("first")]);
                            replies += 1;
                            c.send(&cfg::Serverbound::CustomPayload(p).encode())
                                .unwrap();
                        }
                        cfg::Clientbound::KeepAlive(p) => {
                            c.send(&cfg::Serverbound::KeepAlive(p).encode()).unwrap();
                        }
                        cfg::Clientbound::Disconnect(reason) => {
                            assert!(matches!(case, Case::WrongType | Case::Failure));
                            assert_eq!(*fixture.starts.lock().unwrap(), vec![kind("first")]);
                            let reason =
                                sunrise_protocol::text::plain(&reason.reason, &Default::default());
                            assert!(reason.contains("Configuration error"), "{reason}");
                            break;
                        }
                        cfg::Clientbound::FinishConfiguration(_) => {
                            assert!(!matches!(case, Case::WrongType | Case::Failure));
                            assert_eq!(replies, usize::from(matches!(case, Case::Reply)));
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
                game.leave_server(None).unwrap();
            });
        },
    );
}
