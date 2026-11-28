#![cfg(feature = "client")]

use crate::game::integrated_pair::{IntegratedPair, with_integrated_pair};
use mod_api::{
    BlockUpdateFlags, HandlerThread, Hooks, MenuOpening, MenuRef, MenuSlotBacking, ModEntry,
    PayloadAccepted, PayloadId, Registrar, ToServer, Wire, prelude::BlockStatesHost,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::{
    lan_actions::MenuOpen, native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _,
    native_server::NativeServerGameExt1 as _, player_identity::PlayerIdentityGameExt1 as _,
    transactions::TransactionsGameExt as _,
};
use sunrise_level::containers::BlockEntity;
use sunrise_protocol::menu_extra::{LIVE_DATA_REVISION, OPENING_REVISION};

const POS: (i32, i32, i32) = (1, 64, 1);

#[derive(Wire)]
struct Rename {
    opening: MenuOpening,
    name: String,
}

#[derive(Default)]
struct Witness {
    sender: Mutex<Option<ToServer<Rename>>>,
    resolved: Mutex<Vec<bool>>,
}

struct Register(Arc<Witness>);

#[mod_api::__subscriptions]
impl Hooks for Register {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let witness = self.0.clone();
        let token = r
            .payload_handlers()
            .registrar("1")
            .executes_on(HandlerThread::Main)
            .play_to_server_typed(
                PayloadId("opening_test:rename".into()),
                move |edit: Rename, cx| {
                    let player = cx.player().unwrap();
                    let host = cx.server_host().unwrap();
                    let menu = player.open_menu(host, edit.opening);
                    if let Some(menu) = menu {
                        let Some(MenuSlotBacking::BlockEntity { level, pos, .. }) =
                            menu.slot_backing(host, 0)
                        else {
                            panic!("expected block-backed menu");
                        };
                        let entity = host.block_entity(level, pos).unwrap();
                        assert_eq!(entity.set_name(host, Some(&edit.name)), Ok(true));
                    }
                    witness.resolved.lock().unwrap().push(menu.is_some());
                    Ok(())
                },
            );
        *self.0.sender.lock().unwrap() = Some(token);
    }
}

fn until(
    pair: &mut IntegratedPair<'_, '_, '_>,
    mut ready: impl FnMut(&IntegratedPair<'_, '_, '_>) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ready(pair) {
        assert!(Instant::now() < deadline, "menu traffic did not arrive");
        pair.tick_server(true);
        pair.pump_client();
        std::thread::yield_now();
    }
}

fn current(pair: &IntegratedPair<'_, '_, '_>) -> MenuRef {
    pair.client
        .local_player()
        .unwrap()
        .container_menu(pair.client)
        .unwrap()
}

fn open(pair: &mut IntegratedPair<'_, '_, '_>, conn: u32) -> MenuRef {
    let previous = current(pair);
    pair.call(move |server| assert_eq!(server.open_block_menu(conn, POS), MenuOpen::Opened));
    until(pair, |pair| {
        let menu = current(pair);
        menu != previous
            && pair
                .client
                .client
                .remote
                .as_ref()
                .unwrap()
                .menu
                .as_ref()
                .is_some()
    });
    current(pair)
}

fn wait_for_edit(
    pair: &mut IntegratedPair<'_, '_, '_>,
    witness: &Witness,
    accepted: bool,
    name: &'static str,
) {
    until(pair, |_| !witness.resolved.lock().unwrap().is_empty());
    assert_eq!(
        std::mem::take(&mut *witness.resolved.lock().unwrap()),
        [accepted]
    );
    assert_eq!(
        pair.call(|server| server.world.block_names.get(&POS).cloned()),
        Some(Tag::String(name.into()))
    );
}

#[test]
#[ignore = "daily"]
fn typed_menu_edits_resolve_only_the_current_valid_opening() {
    let witness = Arc::new(Witness::default());
    let mods = vec![ModEntry::new(
        "opening_test",
        Box::new(Register(witness.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_integrated_pair(|pair| {
            until(pair, |pair| {
                pair.client.client.remote.as_ref().unwrap().joined
            });
            let player = pair.client.local_player().unwrap();
            let uuid = player.uuid();
            let conn = pair.call(move |server| {
                let level = server.level("minecraft:overworld").unwrap();
                let chest = BlockStatesHost::block_state_id(server, "minecraft:chest").unwrap();
                assert_eq!(
                    level.set_block(server, [POS.0, POS.1, POS.2], chest, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                assert!(matches!(
                    server.world.block_entities.get(&POS),
                    Some(BlockEntity::Chest(_))
                ));
                let host = server.server.lan.as_ref().unwrap();
                let guest = host.hub.online_players().find(|g| g.uuid == uuid).unwrap();
                assert_eq!(guest.sender.menu_extra_rev, OPENING_REVISION);
                guest.conn
            });
            let inventory = current(pair);
            assert_eq!(inventory.opening(pair.client), None);

            // Use the negotiated rev-2 layout on both endpoints: an ordinary live menu, no token.
            let client_revision = |pair: &mut IntegratedPair<'_, '_, '_>, rev| {
                let remote = pair.client.client.remote.as_mut().unwrap();
                Arc::make_mut(&mut remote.registries).menu_extra_revision = rev;
            };
            let server_revision = |pair: &mut IntegratedPair<'_, '_, '_>, rev| {
                pair.call(move |server| {
                    server
                        .server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(conn, |g| g.sender.menu_extra_rev = rev)
                        .unwrap();
                });
            };
            assert_eq!(
                pair.client
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .registries
                    .menu_extra_revision,
                OPENING_REVISION
            );
            client_revision(pair, LIVE_DATA_REVISION);
            server_revision(pair, LIVE_DATA_REVISION);
            let legacy = open(pair, conn);
            assert_eq!(legacy.opening(pair.client), None);
            client_revision(pair, OPENING_REVISION);
            server_revision(pair, OPENING_REVISION);

            // Current typed input reaches the server and writes to the block entity.
            let menu = open(pair, conn);
            let opening = menu.opening(pair.client).unwrap();
            assert_eq!(legacy.opening(pair.client), None);
            let window = pair.call(move |server| {
                let player = server.player(uuid).unwrap();
                let menu = player.open_menu(server, opening).unwrap();
                assert_eq!(menu.opening(server), Some(opening));
                menu.container_id(server).unwrap()
            });
            let token = witness.sender.lock().unwrap().clone().unwrap();
            let send = |host: &mut dyn mod_api::ClientHost, opening, name: &str| {
                assert_eq!(
                    token.send(
                        host,
                        &Rename {
                            opening,
                            name: name.into()
                        }
                    ),
                    Ok(PayloadAccepted::Queued)
                );
            };
            send(pair.client, opening, "current");
            wait_for_edit(pair, &witness, true, "current");

            // Both operations are journaled in one client transaction, payload first.
            pair.client.storage_scope(|game, _| {
                send(game, opening, "before close");
                assert!(menu.close(game));
                ((), true)
            });
            wait_for_edit(pair, &witness, true, "before close");
            // Same transport, after closing: the host must now see the inventory only.
            send(pair.client, opening, "after close");
            wait_for_edit(pair, &witness, false, "before close");
            assert_eq!(menu.opening(pair.client), None);
            pair.call(move |server| {
                let hub = &mut server.server.lan.as_mut().unwrap().hub;
                hub.update_guest(conn, |g| {
                    assert!(g.menu.is_none());
                    g.next_window = window - 1;
                })
                .unwrap();
            });

            // The same window id now names a different opening.
            let reopened = open(pair, conn).opening(pair.client).unwrap();
            assert_ne!(reopened, opening);
            pair.call(move |server| {
                let player = server.player(uuid).unwrap();
                assert_eq!(
                    player
                        .open_menu(server, reopened)
                        .unwrap()
                        .container_id(server),
                    Some(window)
                );
            });
            send(pair.client, opening, "reused window");
            wait_for_edit(pair, &witness, false, "before close");

            // A spectator is refused even with the current opening.
            let mode = pair.call(move |server| {
                server
                    .server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |g| std::mem::replace(&mut g.game_mode, 3))
                    .unwrap()
            });
            send(pair.client, reopened, "spectator");
            wait_for_edit(pair, &witness, false, "before close");

            // Replace the backing in the same server turn as the resolver assertion,
            // so a periodic invalid-menu close cannot make the assertion pass by itself.
            pair.call(move |server| {
                server
                    .server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |g| g.game_mode = mode)
                    .unwrap();
                let player = server.player(uuid).unwrap();
                let menu = player.open_menu(server, reopened).unwrap();
                server
                    .world
                    .block_entities
                    .replace(POS, BlockEntity::Chest(vec![None; 27]))
                    .unwrap();
                assert_eq!(player.container_menu(server), Some(menu));
                assert_eq!(player.open_menu(server, reopened), None);
            });
            send(pair.client, reopened, "replaced block");
            wait_for_edit(pair, &witness, false, "before close");
        });
    });
}
