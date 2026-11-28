use crate::game::integrated_pair::{IntegratedPair, with_integrated_pair};
use mod_api::{
    Hooks, MenuOpening, MenuRef, ModEntry, PayloadAccepted, PayloadId, Registrar,
    RegistrationContext, ToServer, Wire,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::{
    native_server::NativeServerGameExt1 as _, player_identity::PlayerIdentityGameExt1 as _,
    transactions::TransactionsGameExt as _,
};
use sunrise_level::containers::BlockEntity;
use sunrise_protocol::menu_extra;
use sunrise_sim::lan::menu::Slots;

const AT: (i32, i32, i32) = (1, 64, 0);
const CHANNEL: &str = "menu_opening_probe:edit";

#[derive(Wire)]
struct Edit {
    opening: MenuOpening,
    name: String,
}

#[derive(Default)]
struct Probe {
    send: Mutex<Option<ToServer<Edit>>>,
    received: Mutex<Vec<(String, Option<i32>, bool)>>,
}

struct Registers(Arc<Probe>);
#[mod_api::__subscriptions]
impl Hooks for Registers {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let probe = self.0.clone();
        let send = r.payload_handlers().registrar("1").play_to_server_typed(
            PayloadId(CHANNEL.into()),
            move |edit: Edit, ctx| {
                let player = ctx.player().map_err(|e| format!("sender: {e:?}"))?;
                let host = ctx.server_host().map_err(|e| format!("host: {e:?}"))?;
                let window = player
                    .container_menu(host)
                    .and_then(|m| m.container_id(host));
                let applied = if player.resolve_open_menu(host, edit.opening).is_some() {
                    let level = host.level("minecraft:overworld").ok_or("no overworld")?;
                    let entity = host
                        .block_entity(level, [AT.0, AT.1, AT.2])
                        .ok_or("no chest")?;
                    entity
                        .set_name(host, Some(&edit.name))
                        .map_err(|e| format!("chest name: {e:?}"))?
                } else {
                    false
                };
                probe
                    .received
                    .lock()
                    .unwrap()
                    .push((edit.name, window, applied));
                Ok(())
            },
        );
        *self.0.send.lock().unwrap() = Some(send);
    }
}

fn await_packets(
    pair: &mut IntegratedPair<'_, '_, '_>,
    mut ready: impl FnMut(&mut IntegratedPair<'_, '_, '_>) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready(pair) {
        pair.call(|server| {
            // Drain actual play packets without the subsequent validity tick closing a stale menu.
            server.server.level_pass.packets_drained = false;
            server.tick_lan_packets();
        });
        pair.pump_client();
        assert!(Instant::now() < deadline, "menu packet did not arrive");
        std::thread::yield_now();
    }
}

fn open_chest(
    pair: &mut IntegratedPair<'_, '_, '_>,
    conn: u32,
    uuid: u128,
) -> (MenuRef, i32, MenuOpening) {
    let (window, opening) = pair.call(move |server| {
        let host = server.server.lan.as_mut().unwrap();
        assert_eq!(
            host.hub.guest(conn).unwrap().sender.menu_extra_rev,
            menu_extra::OPENING_REVISION
        );
        host.hub.open_container(
            conn,
            "minecraft:generic_9x3",
            Tag::String("Test chest".into()),
            AT,
            Slots::Block(AT),
            vec![None; 27],
            None,
            &server.common.world,
        );
        let menu = server.player(uuid).unwrap().container_menu(server).unwrap();
        (
            menu.container_id(server).unwrap(),
            menu.opening(server).unwrap(),
        )
    });
    let mut handle = None;
    await_packets(pair, |pair| {
        let client = &*pair.client;
        handle = client
            .local_player()
            .unwrap()
            .container_menu(client)
            .filter(|m| m.opening(client) == Some(opening));
        handle.is_some()
    });
    (handle.unwrap(), window, opening)
}

#[test]
#[ignore = "daily"]
fn typed_menu_edits_resolve_only_the_current_valid_opening() {
    let probe = Arc::new(Probe::default());
    let mods = vec![ModEntry::new(
        "menu_opening_probe",
        Box::new(Registers(probe.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_integrated_pair(|pair| {
            await_packets(pair, |pair| {
                pair.client.client.remote.as_ref().unwrap().joined
            });
            let uuid = pair.client.local_player().unwrap().uuid();
            let conn = pair.call(move |server| {
                let chest = server
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:chest")
                    .unwrap();
                server.world.set(AT.0, AT.1, AT.2, chest).unwrap();
                server
                    .world
                    .block_entities
                    .insert(AT, BlockEntity::Chest(vec![None; 27]));
                server
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .online_players()
                    .find(|g| g.uuid == uuid)
                    .unwrap()
                    .conn
            });
            let send = probe.send.lock().unwrap().clone().unwrap();
            let send_edit = |host: &mut dyn mod_api::ClientHost, opening, name: &str| {
                assert_eq!(
                    send.send(
                        host,
                        &Edit {
                            opening,
                            name: name.into()
                        }
                    ),
                    Ok(PayloadAccepted::Queued)
                );
            };
            let expect = |pair: &mut IntegratedPair<'_, '_, '_>, name: &str, window, applied| {
                await_packets(pair, |_| !probe.received.lock().unwrap().is_empty());
                assert_eq!(
                    *probe.received.lock().unwrap(),
                    [(String::from(name), window, applied)]
                );
                probe.received.lock().unwrap().clear();
            };
            let assert_name = |pair: &mut IntegratedPair<'_, '_, '_>, name: &'static str| {
                pair.call(move |server| {
                    assert_eq!(
                        server.world.block_names.get(&AT),
                        Some(&Tag::String(name.into()))
                    )
                });
            };

            let (menu, window, opening) = open_chest(pair, conn, uuid);
            send_edit(pair.client, opening, "current");
            expect(pair, "current", Some(window), true);
            assert_name(pair, "current");

            pair.client.storage_scope(|client, _| {
                send_edit(client, opening, "before close");
                assert!(menu.close(client));
                assert_eq!(menu.opening(client), Some(opening));
                ((), true)
            });
            assert_eq!(menu.opening(pair.client), None);
            expect(pair, "before close", Some(window), true);
            assert_name(pair, "before close");
            await_packets(pair, |pair| {
                pair.call(move |server| {
                    server
                        .server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(conn)
                        .unwrap()
                        .menu
                        .is_none()
                })
            });

            send_edit(pair.client, opening, "after close");
            expect(pair, "after close", Some(0), false);

            pair.call(move |server| {
                server
                    .server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |g| g.next_window = window - 1);
            });
            let (_, reused, current) = open_chest(pair, conn, uuid);
            assert_eq!(reused, window);
            assert_ne!(current, opening);
            send_edit(pair.client, opening, "previous opening");
            expect(pair, "previous opening", Some(window), false);

            let game_mode = |pair: &mut IntegratedPair<'_, '_, '_>, mode| {
                pair.call(move |server| {
                    server
                        .server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(conn, |g| g.game_mode = mode);
                });
            };
            game_mode(pair, sunrise_sim::lan::guest::SPECTATOR);
            send_edit(pair.client, current, "spectator");
            expect(pair, "spectator", Some(window), false);
            game_mode(pair, sunrise_sim::lan::guest::SURVIVAL);

            pair.call(|server| {
                server
                    .world
                    .block_entities
                    .replace(AT, BlockEntity::Chest(vec![None; 27]))
                    .unwrap();
            });
            send_edit(pair.client, current, "replacement block entity");
            expect(pair, "replacement block entity", Some(window), false);
            assert_name(pair, "before close");
        });
    });
}

#[test]
#[ignore = "daily"]
fn revision_two_negotiates_menu_context_without_an_opening_token() {
    use crate::game::remote::tests::{connect_play_test, spawn_menu_test};
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_protocol::packets::{configuration as cfg, play};

    with_client_game(|game| {
        let (_reader, _writer) = connect_play_test(game, |reader, writer| {
            writer
                .send(
                    &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                        channel: menu_extra::CHANNEL.into(),
                        data: vec![0, menu_extra::LIVE_DATA_REVISION],
                    })
                    .encode(),
                )
                .unwrap();
            let (id, body) = reader.recv().unwrap();
            let cfg::Serverbound::CustomPayload(answer) =
                cfg::Serverbound::decode(id, &body).unwrap()
            else {
                panic!("expected the menu revision answer");
            };
            assert_eq!(answer.channel, menu_extra::CHANNEL);
            assert_eq!(
                menu_extra::read_answer(&answer.data).unwrap(),
                menu_extra::LIVE_DATA_REVISION
            );
        });
        spawn_menu_test(game);
        let remote = game.client.remote.as_ref().unwrap();
        assert_eq!(
            remote.registries.menu_extra_revision,
            menu_extra::LIVE_DATA_REVISION
        );
        let chest = remote
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id("minecraft:generic_9x3")
            .unwrap();
        let player = game.local_player().unwrap();
        assert_eq!(player.container_menu(game).unwrap().opening(game), None);
        game.handle_packet(play::Clientbound::CustomPayload(cfg::CustomPayload {
            channel: menu_extra::CHANNEL.into(),
            data: menu_extra::encode(&menu_extra::Message {
                window: 1,
                pos: Some([AT.0, AT.1, AT.2]),
                data: Vec::new(),
                opening: None,
            })
            .unwrap(),
        }))
        .unwrap();
        game.handle_packet(play::Clientbound::OpenScreen(play::OpenScreen {
            container: 1,
            menu: chest,
            title: Tag::String("Test chest".into()),
        }))
        .unwrap();
        assert_eq!(game.remote_menu().0, 1);
        assert_eq!(player.container_menu(game).unwrap().opening(game), None);
    });
}
