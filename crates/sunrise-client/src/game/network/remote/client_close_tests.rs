use super::super::{C, Game, ItemStack, S, Screen, Tag, play};
use crate::game::{
    native_entity_refs::tests::with_client_game,
    native_hotbar_slot::hotbar_slot_tests::{logged_in, sent},
};

fn isolated(name: &str, body: impl FnOnce()) {
    let (_, module) = module_path!().split_once("::").unwrap();
    let test = format!("{module}::{name}");
    crate::game::in_own_process(&test, body);
}

fn deliver(game: &mut Game<'_>, packet: C) {
    let bytes = packet.encode();
    let mut reader = sunrise_protocol::buf::Reader::new(&bytes);
    game.handle_packet(C::decode(reader.varint().unwrap(), reader.rest()).unwrap())
        .unwrap();
}

fn chest(game: &Game<'_>) -> C {
    open_window(game, 12, "minecraft:generic_9x3")
}

fn open_window(game: &Game<'_>, container: i32, name: &str) -> C {
    C::OpenScreen(play::OpenScreen {
        container,
        menu: game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id(name)
            .unwrap(),
        title: Tag::String("Test menu".into()),
    })
}

fn focus_receiver(game: &mut Game<'_>) -> sunrise_client::widgets::Target {
    let target = sunrise_client::widgets::Target {
        rect: [10.0, 20.0, 80.0, 20.0],
        kind: sunrise_client::widgets::Kind::Button,
    };
    game.key_focus_adopt(target, false);
    target
}

fn sent_close(reader: &mut sunrise_protocol::connection::ConnReader, container: i32) -> S {
    let mut closes = sent(reader).into_iter().filter_map(|packet| match packet {
        S::ContainerClose(close) => Some(close),
        _ => None,
    });
    let close = closes.next().expect("the client sent ContainerClose");
    assert_eq!(close.container, container);
    assert!(closes.next().is_none(), "the window closed only once");
    S::ContainerClose(close)
}

fn with_menu_veto<R>(
    dest: &'static str,
    test: impl FnOnce(&mut Game<'_>, &mut dyn FnMut()) -> R,
) -> R {
    use mod_api::{
        ClientHost, Hooks, ModEntry, ScreenClosingEvent, ScreenInitPostEvent, ScreenOpening,
        ScreenOpeningEvent,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Probe {
        armed: bool,
        events: Vec<(&'static str, &'static str)>,
    }
    struct Veto(&'static str, Arc<Mutex<Probe>>);
    #[mod_api::__subscriptions]
    impl Hooks for Veto {
        fn screen_opening(
            &self,
            _: &dyn ClientHost,
            event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            let mut probe = self.1.lock().unwrap();
            probe.events.push(("open", event.new.id));
            (probe.armed && event.new.id == self.0).then_some(ScreenOpening::Cancel)
        }
        fn screen_closing(&self, _: &dyn ClientHost, event: &mut ScreenClosingEvent<'_>) {
            self.1
                .lock()
                .unwrap()
                .events
                .push(("close", event.screen.id));
        }
        fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            self.1
                .lock()
                .unwrap()
                .events
                .push(("init", event.screen.id));
        }
    }

    let probe = Arc::new(Mutex::new(Probe::default()));
    let mods = vec![ModEntry::new(
        "menu_veto",
        Box::new(Veto(dest, probe.clone())),
    )];
    let mut result = None;
    mod_platform::testing::with_mods(mods.into(), || {
        with_client_game(|game| {
            result = Some(test(game, &mut || {
                *probe.lock().unwrap() = Probe {
                    armed: true,
                    events: Vec::new(),
                };
            }));
        });
    });
    assert_eq!(probe.lock().unwrap().events, [("open", dest)]);
    result.unwrap()
}

#[test]
#[ignore = "daily"]
fn vetoed_chest_open_retires_its_window_on_both_sides() {
    isolated("vetoed_chest_open_retires_its_window_on_both_sides", || {
        use crate::game::{
            native_entity_refs::tests::with_game,
            native_item_places::tests::open_to_lan,
            native_sound_refs::tests::{guest, packets},
        };
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;

        with_game(|server| {
            open_to_lan(server);
            let (guest, rx) = guest(1, 123, [0.5, 65.0, 0.5]);
            server.lan.as_mut().unwrap().hub.guests.push(guest);
            let key = (0, 64, 0);
            server
                .world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = server.level("minecraft:overworld").unwrap();
            let state = server
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            assert_eq!(
                server.level_set_block(
                    level,
                    [0, 64, 0],
                    state,
                    mod_api::BlockUpdateFlags::NEIGHBORS
                ),
                Ok(true)
            );
            server
                .world
                .block_entities
                .get_mut(&key)
                .unwrap()
                .slots_mut()[0] = Some(ItemStack::new("minecraft:apple", 4));
            let _ = packets(&rx);
            server.lan_open_container(1, key);
            let hub = &mut server.lan.as_mut().unwrap().hub;
            let window = hub.guest(1).unwrap().menu.as_ref().unwrap().window;
            assert_ne!(window, 0);
            assert_eq!(server.lid_openers.get(&key), Some(&1));
            let carried = ItemStack::new("minecraft:dirt", 3);
            hub.guests
                .update(0, |guest| guest.carried = Some(carried.clone()));
            hub.resend_menu(1, window, &server.world);
            let mut queued = packets(&rx).into_iter();
            let open = queued
                .find_map(|p| match p {
                    C::OpenScreen(open) => Some(open),
                    _ => None,
                })
                .unwrap();
            assert_eq!(open.container, window);
            let content = queued
                .find_map(|p| match p {
                    C::ContainerSetContent(p) if p.container == window && p.carried.is_some() => {
                        Some(p)
                    }
                    _ => None,
                })
                .expect("matching contents and cursor queued after OpenScreen");
            assert_eq!(content.items[0].as_ref().unwrap().count, 4);
            let close = with_menu_veto("inventory", |client, arm_veto| {
                let mut reader = logged_in(client);
                client.set_screen(Screen::Paused);
                let focus = focus_receiver(client);
                arm_veto();
                let old_carried = Some(ItemStack::new("minecraft:stone", 2));
                client.carried = old_carried.clone();
                client.remote.as_mut().unwrap().state_id = 71;

                deliver(client, C::OpenScreen(open));
                assert_eq!(client.screen, Screen::Paused);
                assert_eq!(client.pad.menu.focus, Some(focus));
                assert_eq!(client.remote_menu().0, 0);
                assert!(client.container.is_none());
                deliver(client, C::ContainerSetContent(content));
                assert_eq!(client.carried, old_carried);
                assert_eq!(client.remote.as_ref().unwrap().state_id, 71);
                sent_close(&mut reader, window)
            });
            server.lan_guest_packet(1, close);
            let guest = server.lan.as_ref().unwrap().hub.guest(1).unwrap();
            assert!(guest.menu.is_none());
            assert_eq!(server.lid_openers.get(&key).copied().unwrap_or(0), 0);
            assert!(
                guest
                    .inventory
                    .iter()
                    .flatten()
                    .any(|stack| stack == &carried)
            );
            assert!(
                !packets(&rx)
                    .iter()
                    .any(|p| matches!(p, C::ContainerClose(_)))
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn vetoed_lectern_open_discards_queued_content_and_keeps_the_book_overlay() {
    isolated(
        "vetoed_lectern_open_discards_queued_content_and_keeps_the_book_overlay",
        || {
            use crate::game::books::BookScreen;
            use sunrise_client::text::Text;

            with_menu_veto("lectern", |game, arm_veto| {
                let mut reader = logged_in(game);
                game.set_screen(Screen::Paused);
                let old_carried = Some(ItemStack::new("minecraft:stone", 2));
                game.carried = old_carried.clone();
                game.remote.as_mut().unwrap().state_id = 71;
                let old_open = open_window(game, 16, "minecraft:lectern");
                deliver(game, old_open);
                assert_eq!(game.remote_menu().0, 16);
                let mut old_book = ItemStack::new("minecraft:writable_book", 1);
                old_book
                    .data_mut()
                    .writable_pages
                    .push("The prior book".into());
                game.remote_lectern_book(Some(old_book));
                let focus = focus_receiver(game);
                arm_veto();

                let window = 17;
                game.remote.as_mut().unwrap().use_target = Some([3, 64, 4]);
                let open = open_window(game, window, "minecraft:lectern");
                deliver(game, open);
                assert_eq!(game.remote_menu().0, 0);
                let book = game
                    .to_network(&Some(ItemStack::new("minecraft:writable_book", 1)))
                    .unwrap();
                let carried = game
                    .to_network(&Some(ItemStack::new("minecraft:dirt", 3)))
                    .unwrap();
                deliver(
                    game,
                    C::ContainerSetContent(play::ContainerSetContent {
                        container: window,
                        state: 4,
                        items: vec![Some(book)],
                        carried: Some(carried),
                    }),
                );
                assert_eq!(game.screen, Screen::Paused);
                assert_eq!(game.pad.menu.focus, Some(focus));
                assert!(
                    matches!(game.book.as_ref(), Some(BookScreen::View { pages, page: 0, lectern: Some((0, 0, 0)) })
                        if pages.as_slice() == [Text::untrusted("The prior book")])
                );
                assert_eq!(game.carried, old_carried);
                assert_eq!(game.remote.as_ref().unwrap().state_id, 71);
                sent_close(&mut reader, window);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn vetoed_mount_open_discards_queued_content_and_keeps_focus() {
    isolated(
        "vetoed_mount_open_discards_queued_content_and_keeps_focus",
        || {
            use sunrise_client::mobs::Mob;
            use sunrise_core::geometry::Vec3;

            with_menu_veto("inventory", |game, arm_veto| {
                let mut reader = logged_in(game);
                let horse = game.res.mob_types().get("minecraft:horse").unwrap().clone();
                let entity = 41;
                game.mobs
                    .push(Mob::new(entity as u32, horse, Vec3::ZERO, 0.0));
                game.set_screen(Screen::Paused);
                let focus = focus_receiver(game);
                arm_veto();

                let old_stack = Some(ItemStack::new("minecraft:stone", 2));
                game.carried = old_stack.clone();
                game.inventory[9] = old_stack.clone();
                game.remote.as_mut().unwrap().state_id = 71;
                let window = 18;
                deliver(
                    game,
                    C::MountScreenOpen(play::MountScreenOpen {
                        container: window,
                        columns: 1,
                        entity,
                    }),
                );
                assert_eq!(game.remote_menu().0, 0);
                assert!(game.mount_menu.opened_mob().is_none());

                let new_stack = game
                    .to_network(&Some(ItemStack::new("minecraft:dirt", 3)))
                    .unwrap();
                let mut items = vec![None; 41];
                items[2] = Some(new_stack.clone()); // First chest slot.
                items[5] = Some(new_stack.clone()); // First player inventory slot.
                deliver(
                    game,
                    C::ContainerSetContent(play::ContainerSetContent {
                        container: window,
                        state: 4,
                        items,
                        carried: Some(new_stack),
                    }),
                );
                assert_eq!(game.screen, Screen::Paused);
                assert_eq!(game.pad.menu.focus, Some(focus));
                assert_eq!(game.carried, old_stack);
                assert_eq!(game.inventory[9], old_stack);
                assert_eq!(game.remote.as_ref().unwrap().state_id, 71);
                sent_close(&mut reader, window);
            });
        },
    );
}

fn close(game: &mut Game<'_>) {
    deliver(
        game,
        C::ContainerClose(play::ContainerClose { container: 37 }),
    );
}

#[test]
#[ignore = "daily"]
fn server_close_restores_inventory_under_chat_without_echo() {
    isolated(
        "server_close_restores_inventory_under_chat_without_echo",
        || {
            with_client_game(|game| {
                let mut reader = logged_in(game);
                let open_chest = chest(game);
                let inventory_carry = ItemStack::new("minecraft:stone", 2);
                for container in [12, 37] {
                    game.carried = Some(inventory_carry.clone());
                    deliver(game, open_chest.clone());
                    assert_eq!(game.screen, Screen::Inventory);
                    game.carried = Some(ItemStack::new("minecraft:dirt", 3));
                    game.begin_chat_screen("draft from a container", false);
                    assert_eq!(game.screen, Screen::Chat);
                    assert_eq!(game.remote_menu().0, 12);
                    game.inv_drag = Some((0, vec![1]));
                    deliver(game, C::ContainerClose(play::ContainerClose { container }));
                    assert_eq!(game.remote_menu().0, 0, "packet id {container}");
                    assert!(
                        game.carried
                            .as_ref()
                            .is_some_and(|stack| stack.item == inventory_carry.item
                                && stack.count == inventory_carry.count)
                    );
                    assert_eq!(game.screen, Screen::Playing);
                    assert!(game.ui.chat_screen.is_none());
                    assert!(game.inv_drag.is_none());
                    game.open_chat_method("");
                    assert_eq!(
                        game.ui.chat_screen.as_ref().unwrap().value(),
                        "draft from a container"
                    );
                    game.close_chat(crate::game::chat::ChatExit::Intentional);
                    assert!(
                        !sent(&mut reader)
                            .iter()
                            .any(|packet| matches!(packet, S::ContainerClose(_)))
                    );
                }

                game.set_screen(Screen::Paused);
                deliver(
                    game,
                    C::ContainerClose(play::ContainerClose { container: 0 }),
                );
                assert_eq!(game.screen, Screen::Playing);
                assert!(
                    !sent(&mut reader)
                        .iter()
                        .any(|p| matches!(p, S::ContainerClose(_)))
                );

                deliver(game, open_chest);
                game.close_container();
                let closes: Vec<_> = sent(&mut reader)
                    .into_iter()
                    .filter_map(|packet| match packet {
                        S::ContainerClose(close) => Some(close.container),
                        _ => None,
                    })
                    .collect();
                assert_eq!(closes, [12]);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn server_close_runs_sign_and_inventory_removal_without_echo() {
    isolated(
        "server_close_runs_sign_and_inventory_removal_without_echo",
        || {
            with_client_game(|game| {
                use sunrise_client::{containers::BlockEntity, signs::SignData};

                let mut reader = logged_in(game);
                game.world
                    .block_entities
                    .insert((1, 65, 2), BlockEntity::Sign(SignData::default()));
                deliver(
                    game,
                    C::OpenSignEditor(play::OpenSignEditor {
                        pos: [1, 65, 2],
                        front: false,
                    }),
                );
                assert!(game.sign_edit.is_some());
                game.sign_input(None, Some("unsent"));
                game.sign_input(Some(winit::keyboard::KeyCode::Enter), None);
                game.sign_input(None, Some("next line"));
                close(game);
                assert!(game.sign_edit.is_none());
                let packets = sent(&mut reader);
                let mut updates = packets.iter().filter_map(|packet| match packet {
                    S::SignUpdate(update) => Some(update),
                    _ => None,
                });
                let sign = updates.next().unwrap();
                assert_eq!(sign.pos, [1, 65, 2]);
                assert!(!sign.front);
                assert_eq!(sign.lines, ["unsent", "next line", "", ""]);
                assert!(updates.next().is_none());
                assert!(!packets.iter().any(|p| matches!(p, S::ContainerClose(_))));

                game.open_player_inventory();
                assert_eq!(game.screen, Screen::Inventory);
                assert_eq!(game.remote_menu().0, 0);
                game.remote.as_mut().unwrap().craft_result =
                    Some(ItemStack::new("minecraft:stick", 4));
                game.craft_grid[0] = Some(ItemStack::new("minecraft:oak_planks", 2));
                close(game);
                assert_eq!(game.screen, Screen::Playing);
                assert!(game.remote.as_ref().unwrap().craft_result.is_none());
                let ingredient = game.craft_grid[0].as_ref().unwrap();
                assert_eq!(ingredient.count, 2);
                assert!(
                    !sent(&mut reader)
                        .iter()
                        .any(|p| matches!(p, S::ContainerClose(_)))
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn server_close_notifies_the_outgoing_receiver_and_only_opens_real_screens() {
    isolated(
        "server_close_notifies_the_outgoing_receiver_and_only_opens_real_screens",
        || {
            use mod_api::{
                ClientHost, Hooks, ModEntry, ScreenClosingEvent, ScreenOpening, ScreenOpeningEvent,
            };
            use std::sync::{Arc, Mutex};

            #[derive(Debug, PartialEq)]
            enum Event {
                Open(&'static str),
                Close {
                    id: &'static str,
                    menu: Option<String>,
                    title: Option<mod_api::Text>,
                    slots: usize,
                },
            }
            struct Observe(Arc<Mutex<Vec<Event>>>);
            #[mod_api::__subscriptions]
            impl Hooks for Observe {
                fn screen_opening(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut ScreenOpeningEvent<'_>,
                ) -> Option<ScreenOpening> {
                    self.0.lock().unwrap().push(Event::Open(event.new.id));
                    // A spurious gameplay Opening would keep an already-deactivated menu.
                    (event.new.id == "playing").then_some(ScreenOpening::Cancel)
                }
                fn screen_closing(&self, _: &dyn ClientHost, event: &mut ScreenClosingEvent<'_>) {
                    self.0.lock().unwrap().push(Event::Close {
                        id: event.screen.id,
                        menu: event.screen.menu.map(str::to_owned),
                        title: event.screen.title.cloned(),
                        slots: event.screen.slots,
                    });
                }
            }
            let events = Arc::new(Mutex::new(Vec::new()));
            let take = || std::mem::take(&mut *events.lock().unwrap());
            let mods = vec![ModEntry::new(
                "screen_probe",
                Box::new(Observe(events.clone())),
            )];
            mod_platform::testing::with_mods(mods.into(), || {
                with_client_game(|game| {
                    let mut reader = logged_in(game);
                    let open = chest(game);
                    deliver(game, open.clone());
                    let slots = game.slot_count();
                    assert!(slots > 36);
                    let title = game.test_observed_title().unwrap().as_ref().clone();
                    take();
                    close(game);
                    assert_eq!(game.screen, Screen::Playing);
                    assert_eq!(
                        take(),
                        [Event::Close {
                            id: "inventory",
                            menu: Some("minecraft:generic_9x3".into()),
                            title: Some(title),
                            slots,
                        }]
                    );

                    deliver(game, open);
                    game.open_sign_editor((1, 65, 2), true);
                    assert_eq!(game.screen, Screen::Inventory);
                    take();
                    close(game);
                    assert_eq!(game.screen, Screen::Playing);
                    assert_eq!(
                        take(),
                        [Event::Close {
                            id: "sign_editor",
                            menu: None,
                            title: None,
                            slots: 0
                        }]
                    );

                    game.set_screen(Screen::Paused);
                    game.player
                        .as_mut()
                        .unwrap()
                        .set_health(0.0, false)
                        .unwrap();
                    game.remote.as_mut().unwrap().show_death_screen = true;
                    let paused = Event::Close {
                        id: "paused",
                        menu: None,
                        title: None,
                        slots: game.slot_count(),
                    };
                    take();
                    close(game);
                    assert_eq!(game.screen, Screen::Dead);
                    assert_eq!(take(), [Event::Open("dead"), paused]);

                    game.remote.as_mut().unwrap().show_death_screen = false;
                    let dead = Event::Close {
                        id: "dead",
                        menu: None,
                        title: None,
                        slots: game.slot_count(),
                    };
                    let _ = sent(&mut reader);
                    close(game);
                    assert_eq!(game.screen, Screen::Playing);
                    assert_eq!(take(), [dead]);
                    assert!(
                        sent(&mut reader)
                            .iter()
                            .any(|p| matches!(p, S::ClientCommand(command) if command.action == 0))
                    );

                    game.set_screen(Screen::Paused);
                    game.remote.as_mut().unwrap().logged_in = false;
                    let paused = Event::Close {
                        id: "paused",
                        menu: None,
                        title: None,
                        slots: game.slot_count(),
                    };
                    take();
                    close(game);
                    assert_eq!(game.title.as_ref().unwrap().mode.id(), "title");
                    assert_eq!(take(), [Event::Open("title"), paused]);
                });
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn server_close_keeps_shown_native_screen_alive_for_closing_and_veto() {
    isolated(
        "server_close_keeps_shown_native_screen_alive_for_closing_and_veto",
        || {
            use mod_api::{
                ClientHost, Hooks, MenuScreen, MenuScreenFactory, MenuScreenLayout, MenuScreenOpen,
                ModEntry, Registrar, RegistrationContext, ScreenClosingEvent, ScreenOpening,
                ScreenOpeningEvent, ScreenSlot,
            };
            use std::sync::{Arc, Mutex};
            use sunrise_client::{hud::HudBatch, menu::Input};
            use sunrise_protocol::{
                menu_extra::{self, CHANNEL, Message},
                packets::configuration::CustomPayload,
                registry::Ids,
            };

            const MENU: &str = "test:close_native";
            const POS: [i32; 3] = [11, 64, 19];
            const SLOT_POS: [f32; 2] = [12.0, 24.0];

            #[derive(Default)]
            struct Probe {
                cancel_dead: bool,
                cancel_native_open: bool,
                closing: Vec<(Option<String>, Option<[i32; 3]>, Option<[f32; 2]>)>,
                draws: Vec<Vec<i32>>,
            }
            type Shared = Arc<Mutex<Probe>>;

            struct Factory(Shared);
            impl MenuScreenFactory for Factory {
                fn create(&self, _: &dyn ClientHost, _: MenuScreenOpen<'_>) -> Option<MenuScreen> {
                    let draws = self.0.clone();
                    Some(
                        MenuScreen::new(MenuScreenLayout {
                            width: 176.0,
                            height: 166.0,
                            inventory_x: 8.0,
                            inventory_y: 84.0,
                            slots: vec![
                                Some(ScreenSlot {
                                    position: SLOT_POS,
                                    ..Default::default()
                                }),
                                None,
                            ],
                            own_labels: true,
                            own_background: true,
                            ..Default::default()
                        })
                        .with_draw(move |_, frame, _| {
                            draws.lock().unwrap().draws.push(frame.data.to_vec());
                        }),
                    )
                }
            }

            struct Listener(Shared);
            #[mod_api::__subscriptions]
            impl Hooks for Listener {
                fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                    r.register_menu_screen(MENU, Arc::new(Factory(self.0.clone())))
                        .unwrap();
                }
                fn screen_opening(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut ScreenOpeningEvent<'_>,
                ) -> Option<ScreenOpening> {
                    let probe = self.0.lock().unwrap();
                    ((event.new.id == "dead" && probe.cancel_dead)
                        || (event.new.menu == Some(MENU) && probe.cancel_native_open))
                        .then_some(ScreenOpening::Cancel)
                }
                fn screen_closing(
                    &self,
                    host: &dyn ClientHost,
                    event: &mut ScreenClosingEvent<'_>,
                ) {
                    self.0.lock().unwrap().closing.push((
                        event.screen.menu.map(str::to_owned),
                        host.screen_pos(),
                        host.screen_slot(MENU, 0),
                    ));
                }
            }

            let probe = Shared::default();
            let mods: mod_api::Mods = vec![ModEntry::new(
                "close_native_probe",
                Box::new(Listener(probe.clone())),
            )]
            .into();
            sunrise_client::containers::with_mod_menu_types(
                [(
                    MENU.into(),
                    "close_native_probe".into(),
                    2,
                    2,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    false,
                    None,
                )],
                || {
                    mod_platform::testing::with_mods(mods.clone(), || {
                        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
                        with_client_game(|game| {
                            let mut reader = logged_in(game);
                            Arc::make_mut(&mut game.remote.as_mut().unwrap().builtin)
                                .registries
                                .insert(
                                    "minecraft:menu".into(),
                                    Ids::from_names(vec![MENU.to_owned()]),
                                );

                            let open_native = |game: &mut Game<'_>, window: i32| {
                                deliver(
                                    game,
                                    C::CustomPayload(CustomPayload {
                                        channel: CHANNEL.into(),
                                        data: menu_extra::encode(&Message {
                                            window,
                                            pos: Some(POS),
                                            data: Vec::new(),
                                        })
                                        .unwrap(),
                                    }),
                                );
                                deliver(
                                    game,
                                    C::OpenScreen(play::OpenScreen {
                                        container: window,
                                        menu: 0,
                                        title: Tag::String("Native".into()),
                                    }),
                                );
                            };

                            // 1. Decoded server ContainerClose keeps live reads valid during Closing.
                            open_native(game, 14);
                            assert_eq!(game.screen, Screen::Inventory);
                            deliver(
                                game,
                                C::ContainerSetData(play::ContainerSetData {
                                    container: 14,
                                    id: 0,
                                    value: 42,
                                }),
                            );
                            let mut batch = HudBatch::scratch([320, 240]);
                            assert!(game.draw_native_screen(&mut batch, 0.0, 0.0));
                            assert_eq!(probe.lock().unwrap().draws, [vec![42]]);

                            deliver(
                                game,
                                C::ContainerClose(play::ContainerClose { container: 14 }),
                            );
                            assert_eq!(game.screen, Screen::Playing);
                            assert_eq!(game.remote_menu().0, 0);
                            assert_eq!(
                                probe.lock().unwrap().closing,
                                [(Some(MENU.to_owned()), Some(POS), Some(SLOT_POS))]
                            );
                            assert!(!game.draw_native_screen(&mut batch, 0.0, 0.0));
                            assert_eq!(game.screen_pos(), None);
                            assert_eq!(game.screen_slot(MENU, 0), None);

                            // 2. Vetoing a real destination (DeathScreen) keeps the shown screen
                            //    drawing and readable while rejecting stale window protocol ops.
                            probe.lock().unwrap().closing.clear();
                            probe.lock().unwrap().draws.clear();
                            probe.lock().unwrap().cancel_dead = true;
                            open_native(game, 15);
                            deliver(
                                game,
                                C::ContainerSetData(play::ContainerSetData {
                                    container: 15,
                                    id: 0,
                                    value: 7,
                                }),
                            );
                            game.player
                                .as_mut()
                                .unwrap()
                                .set_health(0.0, false)
                                .unwrap();
                            game.remote.as_mut().unwrap().show_death_screen = true;
                            let _ = sent(&mut reader);
                            deliver(
                                game,
                                C::ContainerClose(play::ContainerClose { container: 15 }),
                            );

                            assert_eq!(game.screen, Screen::Inventory);
                            assert_eq!(game.remote_menu().0, 0);
                            assert!(probe.lock().unwrap().closing.is_empty());
                            assert_eq!(game.screen_pos(), Some(POS));
                            assert_eq!(game.screen_slot(MENU, 0), Some(SLOT_POS));
                            // Stale protocol/menu ops for the closed window are rejected.
                            deliver(
                                game,
                                C::ContainerSetData(play::ContainerSetData {
                                    container: 15,
                                    id: 0,
                                    value: 99,
                                }),
                            );
                            assert!(!game.remote_button_click(1));
                            assert!(!game.remote_menu_ops(&[(Some(0), 0, Input::Pickup)]));
                            assert!(!sent(&mut reader).iter().any(|p| matches!(
                                p,
                                S::ContainerClick(_) | S::ContainerButtonClick(_)
                            )));
                            // The displayed native screen still draws with its pre-close data.
                            assert!(game.draw_native_screen(&mut batch, 0.0, 0.0));
                            assert_eq!(probe.lock().unwrap().draws, [vec![7]]);

                            // Leaving the vetoed screen later closes it with live reads and no echo.
                            probe.lock().unwrap().cancel_dead = false;
                            game.set_screen(Screen::Playing);
                            assert_eq!(game.screen, Screen::Playing);
                            assert_eq!(
                                probe.lock().unwrap().closing,
                                [(Some(MENU.to_owned()), Some(POS), Some(SLOT_POS))]
                            );
                            assert!(!game.draw_native_screen(&mut batch, 0.0, 0.0));
                            assert!(
                                !sent(&mut reader)
                                    .iter()
                                    .any(|p| matches!(p, S::ContainerClose(_)))
                            );

                            // 3. A never-shown native opening doesn't own the pause screen's focus.
                            game.set_screen(Screen::Paused);
                            let focus = focus_receiver(game);
                            {
                                let mut probe = probe.lock().unwrap();
                                probe.closing.clear();
                                probe.cancel_native_open = true;
                            }
                            open_native(game, 16);
                            assert_eq!(game.screen, Screen::Paused);
                            assert_eq!(game.pad.menu.focus, Some(focus));
                            assert_eq!(game.remote_menu().0, 0);
                            assert!(game.active_native_screen().is_none());
                            assert!(game.mod_screen.is_none());
                            assert!(probe.lock().unwrap().closing.is_empty());
                            sent_close(&mut reader, 16);
                        });
                    });
                },
            );
        },
    );
}
