use super::{connect_play_test, spawn_menu_test};
use crate::game::{
    native_entity_refs::tests::with_client_game, remote::Received, viewed_worlds::tests::column,
};
use mod_api::{
    HandlerThread, Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes,
    PayloadContextError, PayloadHandleError, PayloadId, PayloadWork, PayloadWorkStatus, Registrar,
};
use std::{
    sync::{Arc, Mutex, mpsc},
    thread::{self, ThreadId},
    time::{Duration, Instant},
};
use sunrise_client::{net::Event, raw_payloads};
use sunrise_protocol::{
    packets::{
        Empty, configuration as cfg,
        custom_payload::CustomPayload,
        handshake,
        play::{self, Clientbound as C, Serverbound as S},
    },
    payload_negotiation::{CHANNEL, Control},
};

const MAIN: &str = "bundle_probe:main";
const NETWORK: &str = "bundle_probe:network";

struct Witness {
    main: ThreadId,
    order: Mutex<Vec<&'static str>>,
    transfer_started: Mutex<Option<mpsc::Receiver<()>>>,
}

struct Handler {
    witness: Arc<Witness>,
    network: bool,
}

impl IPayloadHandler for Handler {
    fn handle(
        &self,
        bytes: PayloadBytes,
        ctx: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        if self.network {
            assert_eq!(thread::current().id(), self.witness.main);
            assert!(matches!(
                ctx.client_host(),
                Err(PayloadContextError::WrongThread)
            ));
            {
                if bytes.0 == [4] {
                    self.witness
                        .transfer_started
                        .lock()
                        .unwrap()
                        .take()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .expect("new connection starts before the next member");
                    self.witness
                        .order
                        .lock()
                        .unwrap()
                        .push("network after transfer");
                    return Ok(());
                }
                let mut order = self.witness.order.lock().unwrap();
                if bytes.0 == [3] {
                    order.push("network after close");
                    return Ok(());
                }
                assert_eq!(*order, ["main"]);
                order.push("network");
            }
            assert_eq!(
                ctx.enqueue_work(Arc::new(Work(self.witness.clone()))),
                Ok(PayloadWorkStatus::Queued)
            );
        } else {
            assert_eq!(thread::current().id(), self.witness.main);
            assert!(ctx.client_host().is_ok());
            self.witness.order.lock().unwrap().push("main");
        }
        let id = if self.network { NETWORK } else { MAIN };
        ctx.reply(&PayloadId(id.into()), bytes).unwrap();
        Ok(())
    }
}

struct Work(Arc<Witness>);
impl PayloadWork for Work {
    fn run(&self, ctx: &mut dyn IPayloadContext) -> Result<(), String> {
        assert_eq!(thread::current().id(), self.0.main);
        assert!(ctx.client_host().is_ok());
        self.0.order.lock().unwrap().push("work");
        ctx.reply(&PayloadId(NETWORK.into()), PayloadBytes(vec![9]))
            .unwrap();
        Ok(())
    }
}

struct Probe(Arc<Witness>);
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for (id, network) in [(MAIN, false), (NETWORK, true)] {
            let handler = Arc::new(Handler {
                witness: self.0.clone(),
                network,
            });
            registrar
                .payload_handlers()
                .registrar("1")
                .executes_on(if network {
                    HandlerThread::Network
                } else {
                    HandlerThread::Main
                })
                .play_bidirectional(PayloadId(id.into()), handler.clone(), Some(handler));
        }
    }
}

fn wait_for_bundle(game: &mut crate::game::Game<'_>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let remote = game.remote.as_mut().unwrap();
    let mut backlog = std::mem::take(&mut remote.backlog);
    loop {
        remote.receive_credited_backlog(&mut backlog);
        if matches!(backlog.back(), Some(Received::Event(Event::Bundle(_), _))) {
            break;
        }
        assert!(Instant::now() < deadline, "bundle did not reach main");
        thread::yield_now();
    }
    remote.backlog = backlog;
}

fn with_probe(test: impl FnOnce(&mut crate::game::Game<'_>, Arc<Witness>)) {
    let witness = Arc::new(Witness {
        main: thread::current().id(),
        order: Mutex::new(Vec::new()),
        transfer_started: Mutex::new(None),
    });
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "bundle_probe",
            Box::new(Probe(witness.clone())),
        )]
        .into(),
        || with_client_game(|game| test(game, witness)),
    );
}

fn connect_probe(
    game: &mut crate::game::Game<'_>,
) -> (
    Arc<raw_payloads::Local>,
    sunrise_protocol::connection::ConnReader,
    sunrise_protocol::connection::ConnWriter,
) {
    let local = raw_payloads::Local::capture(raw_payloads::Side::Server).unwrap();
    let (reader, writer) = connect_play_test(game, |reader, writer| {
        writer
            .send(
                &cfg::Clientbound::CustomPayload(CustomPayload {
                    channel: CHANNEL.into(),
                    data: Control::Offer(local.manifest.clone()).encode().unwrap(),
                })
                .encode(),
            )
            .unwrap();
        let (id, body) = reader.recv().unwrap();
        assert!(
            matches!(cfg::Serverbound::decode(id, &body).unwrap(), cfg::Serverbound::CustomPayload(p) if p.channel == CHANNEL)
        );
    });
    spawn_menu_test(game);
    (local, reader, writer)
}

#[test]
#[ignore = "daily"]
fn a_bundle_applies_its_column_and_orders_main_network_and_close() {
    with_probe(|game, witness| {
        let (local, mut reader, mut writer) = connect_probe(game);
        let block = |x| {
            C::BlockUpdate(play::BlockUpdate {
                pos: [x, 70, 5],
                state: 0,
            })
        };
        // The same packets handed individually to the vanilla-order handlers.
        game.handle_packet(column(-1, 0)).unwrap();
        assert_eq!(game.world.block(-13, 70, 5).0, 1);
        game.handle_packet(block(-13)).unwrap();
        let expected = [game.world.block(-13, 70, 5), game.world.block(-13, 71, 5)];
        assert_eq!(expected.map(|state| state.0), [0, 1]);
        let delimiter = C::BundleDelimiter(Empty);
        let payload = |channel: &str, x| {
            C::CustomPayload(CustomPayload {
                channel: channel.into(),
                data: vec![x],
            })
        };
        for packet in [
            delimiter.clone(),
            column(0, 0),
            block(3),
            payload(MAIN, 1),
            payload(NETWORK, 2),
            delimiter.clone(),
        ] {
            writer
                .send(
                    &packet
                        .try_encode_with_payload_codecs(&local.codecs)
                        .unwrap(),
                )
                .unwrap();
        }

        // Receiving the completed turn alone applies neither the column
        // nor any payload handler on the network thread.
        wait_for_bundle(game);
        assert!(witness.order.lock().unwrap().is_empty());
        assert!(!game.world.loaded(0, 0));
        game.poll_remote_events(true).unwrap();
        assert_eq!(
            [game.world.block(3, 70, 5), game.world.block(3, 71, 5)],
            expected
        );
        assert_eq!(*witness.order.lock().unwrap(), ["main", "network", "work"]);
        for (channel, value) in [(MAIN, 1), (NETWORK, 2), (NETWORK, 9)] {
            loop {
                let (id, body) = reader.recv().unwrap();
                if let S::CustomPayload(p) =
                    S::decode_with_payload_codecs(id, &body, &local.codecs).unwrap()
                {
                    assert_eq!(
                        (p.channel.as_str(), p.data.as_slice()),
                        (channel, [value].as_slice())
                    );
                    break;
                }
            }
        }

        // A Disconnect member closes the connection, not the iteration.
        // A cookie and a typed handler still run before the listener is removed.
        let cookies = game.remote.as_ref().unwrap().session.cookies.clone();
        let cookie = "bundle_probe:after_close";
        for packet in [
            delimiter.clone(),
            C::Disconnect(cfg::Disconnect {
                reason: sunrise_core::nbt::Tag::String("bundle closed".into()),
            }),
            C::StoreCookie(cfg::StoreCookie {
                key: cookie.into(),
                payload: vec![17],
            }),
            payload(NETWORK, 3),
            delimiter,
        ] {
            writer
                .send(
                    &packet
                        .try_encode_with_payload_codecs(&local.codecs)
                        .unwrap(),
                )
                .unwrap();
        }
        wait_for_bundle(game);
        game.poll_remote_events(true).unwrap();
        assert!(game.remote.is_none());
        assert_eq!(cookies.lock().unwrap().get(cookie), Some(&vec![17]));
        assert_eq!(
            witness.order.lock().unwrap().last(),
            Some(&"network after close")
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_bundled_transfer_starts_the_next_session_before_later_members() {
    with_probe(|game, witness| {
        let (local, _reader, mut writer) = connect_probe(game);
        // The source uses an in-memory wire for this fixture; let it transfer
        // like a multiplayer listener rather than the integrated local world.
        game.remote.as_mut().unwrap().session.address = "source.invalid".into();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let destination = listener.local_addr().unwrap();
        let (started, observed) = mpsc::channel();
        *witness.transfer_started.lock().unwrap() = Some(observed);
        let target = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut target = sunrise_protocol::connection::Connection::from_stream(stream).unwrap();
            let (id, body) = target.recv().unwrap();
            let handshake::Serverbound::Intention(packet) =
                handshake::Serverbound::decode(id, &body).unwrap()
            else {
                panic!("expected transfer handshake")
            };
            assert_eq!(packet.intent, handshake::TRANSFER);
            started.send(()).unwrap();
        });
        let cookie = "bundle_probe:transfer";
        for packet in [
            C::BundleDelimiter(Empty),
            C::Transfer(cfg::Transfer {
                host: destination.ip().to_string(),
                port: destination.port().into(),
            }),
            C::CustomPayload(CustomPayload {
                channel: NETWORK.into(),
                data: vec![4],
            }),
            C::StoreCookie(cfg::StoreCookie {
                key: cookie.into(),
                payload: vec![27],
            }),
            C::BundleDelimiter(Empty),
        ] {
            writer
                .send(
                    &packet
                        .try_encode_with_payload_codecs(&local.codecs)
                        .unwrap(),
                )
                .unwrap();
        }
        wait_for_bundle(game);
        game.poll_remote_events(true).unwrap();
        assert_eq!(*witness.order.lock().unwrap(), ["network after transfer"]);
        let next = &game.remote.as_ref().unwrap().session;
        assert_eq!(next.address, destination.to_string());
        assert_eq!(next.cookies.lock().unwrap().get(cookie), Some(&vec![27]));
        target.join().unwrap();
    });
}
