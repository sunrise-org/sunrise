//! `menu_still_valid` through a real `MenuPart` on a server's guest: the
//! per-tick close and the click check ask it for block and blockless mod menus. Vanilla menus
//! (even with a part attached) and chest vehicles never ask.

use mod_api::{Hooks, MenuPart, MenuStillValidEvent, ModEntry, Mods, Registrar};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use sunrise_level::{
    chunk::Chunk,
    containers::{BlockEntity, ModMenuType},
};
use sunrise_protocol::packets::play::{ContainerButtonClick, ContainerClick, Serverbound};
use sunrise_sim::lan::menu::{Menu, Slots, VEHICLE_KEY_Y, asks_still_valid};

/// What the part answers, and how often it was asked.
static KEEP: AtomicBool = AtomicBool::new(true);
static ASKED: AtomicUsize = AtomicUsize::new(0);
/// How often the part on the vanilla chest menu was asked: never, whatever it would answer.
static VANILLA_ASKED: AtomicUsize = AtomicUsize::new(0);

struct Shell;
#[mod_api::sunrise_part]
impl MenuPart for Shell {
    fn menu_still_valid(&self, _: &dyn mod_api::LoopContext, event: &mut MenuStillValidEvent<'_>) {
        ASKED.fetch_add(1, Ordering::Relaxed);
        assert_eq!(event.menu, "test:shell");
        event.valid = KEEP.load(Ordering::Relaxed);
    }
}
/// A part on a vanilla menu type, which would keep any chest open if asked.
struct VanillaChest;
#[mod_api::sunrise_part]
impl MenuPart for VanillaChest {
    fn menu_still_valid(&self, _: &dyn mod_api::LoopContext, event: &mut MenuStillValidEvent<'_>) {
        VANILLA_ASKED.fetch_add(1, Ordering::Relaxed);
        event.valid = true;
    }
}
struct Mod;
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("test:shell", Shell);
        r.menu_part("minecraft:generic_9x3", VanillaChest);
    }
}

fn scoped_dispatch() -> impl Drop {
    sunrise_sim::native_client_hooks::scoped_menu_validity_hooks(
        sunrise_gameplay_common::game::api_values::menu_still_valid,
        |menu| mod_api::__private::menu_listens(mod_api::HookId::MenuStillValid, menu),
    )
}

#[test]
#[ignore = "daily"]
fn a_mod_menu_part_decides_whether_its_menu_stays_open() {
    let mods: Mods = vec![ModEntry::new("shell", Box::new(Mod))].into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let _callbacks = scoped_dispatch();
    let shell: &'static ModMenuType = Box::leak(Box::new(ModMenuType {
        id: "test:shell",
        owner: "test",
        slots: 27,
        player_inventory: true,
        data_slots: 0,
        owned_slots: None,
        results: Vec::new(),
        inputs: Vec::new(),
        crafting: None,
        links: None,
    }));
    mod_platform::testing::with_mods(mods, || {
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let eye = game.server.lan.as_ref().unwrap().hub.guests[0].position;
            // 20 blocks from the guest: past reach + 4, so vanilla closes it.
            let far = (eye[0] as i32, eye[1] as i32, eye[2] as i32 + 20);
            let beside = (far.0 + 1, far.1, far.2);
            // 3 blocks away: in reach.
            let near = (eye[0] as i32, eye[1] as i32, eye[2] as i32 + 3);
            for k in [far, beside, near] {
                game.world
                    .block_entities
                    .insert(k, BlockEntity::Chest(vec![None; 27]));
            }
            let open = |game: &mut crate::game::ServerGame<'_>, slots: Slots, modded: bool| {
                let pos = match slots {
                    Slots::Block(k) | Slots::Double(k, _) => k,
                    _ => far,
                };
                let host = game.server.lan.as_mut().unwrap();
                host.hub.open_container(
                    12,
                    "minecraft:generic_9x3",
                    sunrise_core::nbt::Tag::String("test".into()),
                    pos,
                    slots,
                    vec![None; 27],
                    None,
                );
                host.hub.guests.update(0, |guest| {
                    let menu = guest.menu.as_mut().unwrap();
                    if modded {
                        menu.registry_id = "test:shell".into();
                        menu.mod_type = Some(shell);
                    }
                    menu.asks_still_valid = asks_still_valid(menu.mod_type, &menu.registry_id);
                });
            };
            let is_open = |game: &crate::game::ServerGame<'_>| {
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .is_some()
            };
            let click = |game: &mut crate::game::ServerGame<'_>| {
                let window = game.server.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .as_ref()
                    .unwrap()
                    .window;
                game.lan_guest_packet(
                    12,
                    Serverbound::ContainerClick(ContainerClick {
                        container: window,
                        state: 0,
                        slot: -999,
                        button: 0,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    }),
                );
            };
            let asked = || ASKED.load(Ordering::Relaxed);

            // A vanilla chest keeps vanilla's answer although a part is attached to
            // its menu type: open in reach, closed out of it, and that part never asked.
            assert!(mod_api::__private::menu_listens(
                mod_api::HookId::MenuStillValid,
                "minecraft:generic_9x3"
            ));
            open(game, Slots::Block(near), false);
            game.lan_containers_tick();
            assert!(is_open(game));
            game.guest_close_all(12);
            open(game, Slots::Block(far), false);
            game.lan_containers_tick();
            assert!(!is_open(game));
            assert_eq!(asked(), 0);
            assert_eq!(VANILLA_ASKED.load(Ordering::Relaxed), 0);

            // The mod's part keeps its far menu open through ticks and clicks.
            KEEP.store(true, Ordering::Relaxed);
            open(game, Slots::Block(far), true);
            game.lan_containers_tick();
            click(game);
            game.lan_containers_tick();
            assert!(is_open(game));
            assert!(asked() >= 3);

            // And closes it: on the tick, and on a click before any tick.
            KEEP.store(false, Ordering::Relaxed);
            game.lan_containers_tick();
            assert!(!is_open(game));
            open(game, Slots::Block(far), true);
            click(game);
            assert!(!is_open(game));

            // Only a button click for the open window asks: a stale window's click leaves the
            // menu be, and the open window's closes it.
            open(game, Slots::Block(far), true);
            let window = game.server.lan.as_ref().unwrap().hub.guests[0]
                .menu
                .as_ref()
                .unwrap()
                .window;
            let button = |container| {
                Serverbound::ContainerButtonClick(ContainerButtonClick {
                    container,
                    button: 0,
                })
            };
            game.lan_guest_packet(12, button(window + 1));
            assert!(is_open(game));
            game.lan_guest_packet(12, button(window));
            assert!(!is_open(game));

            // A double chest's menu asks too.
            KEEP.store(true, Ordering::Relaxed);
            let before = asked();
            open(game, Slots::Double(far, beside), true);
            game.lan_containers_tick();
            assert!(is_open(game));
            assert!(asked() > before);

            // An always-true part doesn't hold it past death and respawn, or a disconnect.
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |guest| guest.dead = true);
            game.guest_respawn(12);
            assert!(!is_open(game));
            open(game, Slots::Block(far), true);
            game.guest_disconnected(12);
            assert!(!is_open(game));

            // A chest vehicle's container is kept under a key, not a block: never asked.
            let vehicle = (7, VEHICLE_KEY_Y, 0);
            game.world
                .block_entities
                .insert(vehicle, BlockEntity::Chest(vec![None; 27]));
            let menu = Menu {
                window: 1,
                active_chest: None,
                kind: 2,
                pos: Some(vehicle),
                slots: Slots::Block(vehicle),
                size: 27,
                state: 0,
                sent: Vec::new(),
                drag: None,
                sent_data: Vec::new(),
                reported_unsent_data_len: None,
                blockless_data: Vec::new(),

                mod_type: Some(shell),
                registry_id: "test:shell".into(),
                craft: None,
                links: Vec::new(),
                asks_still_valid: asks_still_valid(Some(shell), "test:shell"),
                opening_item: None,
                opener_block: None,
                removed: false,
                slot_changed_seen: None,
            };
            assert!(menu.asks_still_valid);
            let before = asked();
            let host = game.server.lan.as_ref().unwrap();
            assert!(menu.still_valid(&host.hub.loop_context, &game.world, &host.hub.guests[0]));
            assert_eq!(asked(), before);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_blockless_mod_menu_closes_when_its_part_answers_false() {
    // Use a prepared registry slot for a synthetic mod menu definition.
    const MENU: &str = "minecraft:generic_9x1";
    #[derive(Default)]
    struct Answer {
        keep: AtomicBool,
        asks: AtomicUsize,
    }
    struct Blockless(Arc<Answer>);
    #[mod_api::sunrise_part]
    impl MenuPart for Blockless {
        fn menu_still_valid(
            &self,
            _: &dyn mod_api::LoopContext,
            event: &mut MenuStillValidEvent<'_>,
        ) {
            assert_eq!(event.menu, MENU);
            assert_eq!(
                event.pos, None,
                "the screen position is not a backing block"
            );
            assert!(event.valid, "a blockless menu starts valid");
            self.0.asks.fetch_add(1, Ordering::Relaxed);
            event.valid = self.0.keep.load(Ordering::Relaxed);
        }
    }

    struct BlocklessMod(Arc<Answer>);
    #[mod_api::__subscriptions]
    impl Hooks for BlocklessMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            r.menu_part(MENU, Blockless(self.0.clone()));
        }
    }

    let answer = Arc::new(Answer::default());
    answer.keep.store(true, Ordering::Relaxed);
    let mods: Mods = vec![ModEntry::new(
        "blockless",
        Box::new(BlocklessMod(answer.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let _callbacks = scoped_dispatch();
        sunrise_level::containers::with_mod_menu_types(
            [(
                MENU.into(),
                "test".into(),
                9,
                0,
                Vec::new(),
                Vec::new(),
                None,
                None,
                true,
                None,
            )],
            || {
                super::super::native_guest_turn_tests::with_guests(|game| {
                    // A screen position is not a vehicle key, even at this reserved Y.
                    let pos = Some((0, sunrise_gameplay_common::game::minecarts::CART_KEY_Y, 100));
                    let open = |game: &mut crate::game::ServerGame<'_>| {
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .open_menu_window(
                                12,
                                MENU,
                                sunrise_core::nbt::Tag::String("test".into()),
                                pos,
                                Slots::Native,
                                vec![None; 9],
                                None,
                            )
                            .unwrap()
                    };
                    let window_of_guest = |game: &crate::game::ServerGame<'_>| {
                        game.server.lan.as_ref().unwrap().hub.guests[0]
                            .menu
                            .as_ref()
                            .map(|menu| menu.window)
                    };
                    let old_window = open(game);
                    game.lan_containers_tick();
                    assert_eq!(window_of_guest(game), Some(old_window));

                    answer.keep.store(false, Ordering::Relaxed);
                    game.lan_containers_tick();
                    assert_eq!(window_of_guest(game), None);

                    let click = |game: &mut crate::game::ServerGame<'_>, container| {
                        game.lan_guest_packet(
                            12,
                            Serverbound::ContainerClick(ContainerClick {
                                container,
                                state: 0,
                                slot: -999,
                                button: 0,
                                input: 0,
                                changed: Vec::new(),
                                carried: None,
                            }),
                        );
                    };
                    let window = open(game);
                    let before = answer.asks.load(Ordering::Relaxed);
                    for stale in [0, old_window] {
                        click(game, stale);
                        assert_eq!(window_of_guest(game), Some(window));
                        assert_eq!(answer.asks.load(Ordering::Relaxed), before);
                    }

                    let state_of_guest = |game: &crate::game::ServerGame<'_>| {
                        game.server.lan.as_ref().unwrap().hub.guests[0]
                            .menu
                            .as_ref()
                            .unwrap()
                            .state
                    };
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |g| g.game_mode = 3);
                    let old_state = state_of_guest(game);
                    click(game, window);
                    assert_eq!(window_of_guest(game), Some(window));
                    assert_eq!(state_of_guest(game), (old_state + 1) & 0x7FFF);
                    assert_eq!(answer.asks.load(Ordering::Relaxed), before);

                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |g| g.game_mode = 0);
                    click(game, window);
                    assert_eq!(window_of_guest(game), None);
                    assert_eq!(answer.asks.load(Ordering::Relaxed), before + 1);
                    assert!(
                        game.server
                            .lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .native_menu_events
                            .is_empty()
                    );
                });
            },
        );
    });
}

/// Opening a physical ender chest through the server, then replacing its block
/// entity while the guest stays in reach: the menu closes, a stale click moves
/// nothing, and the replacement gets no close effects (`activeChest` identity).
#[test]
#[ignore = "daily"]
fn an_ender_chest_replaced_in_place_retires_its_menu_without_close_effects() {
    use sunrise_protocol::{
        buf::Reader,
        packets::play::{ContainerClick, Serverbound},
    };
    use sunrise_sim::lan::{Out, PlayerSender};

    mod_platform::__private::test_platform::install();
    super::super::native_guest_turn_tests::with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let guest_pos = game.server.lan.as_ref().unwrap().hub.guests[0].position;
        let pos = (
            guest_pos[0] as i32,
            guest_pos[1] as i32,
            guest_pos[2] as i32 + 3,
        );
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:ender_chest")
            .unwrap();
        game.world.set(pos.0, pos.1, pos.2, chest).unwrap();
        game.world
            .block_entities
            .insert(pos, BlockEntity::Chest(vec![None; 27]));
        let (tx, rx) = std::sync::mpsc::channel();
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.sender = PlayerSender::new(tx, Arc::default());
            g.ender_items[0] = Some(sunrise_sim::items::ItemStack::new(
                "minecraft:diamond",
                1,
            ));
        });

        assert_eq!(
            game.lan_open_container(12, pos),
            sunrise_gameplay_common::game::lan_actions::MenuOpen::Opened
        );
        let old = {
            let g = &game.server.lan.as_ref().unwrap().hub.guests[0];
            let menu = g.menu.as_ref().expect("the ender chest menu opened");
            assert_eq!(menu.slots, Slots::Ender);
            assert!(menu.active_chest.is_some());
            menu.window
        };
        // Drain the opening effects.
        let drain = |rx: &std::sync::mpsc::Receiver<Out>| {
            rx.try_iter().for_each(|_| {});
        };
        drain(&rx);

        // Replace the block entity at the same position while the guest stays in reach.
        game.world.block_entities.remove(&pos);
        game.world
            .block_entities
            .insert(pos, BlockEntity::Chest(vec![None; 27]));
        game.lan_containers_tick();
        assert!(
            game.server.lan.as_ref().unwrap().hub.guests[0]
                .menu
                .is_none(),
            "the replaced ender chest closes the menu"
        );

        // A stale click on the old window moves nothing.
        game.lan_guest_packet(
            12,
            Serverbound::ContainerClick(ContainerClick {
                container: old,
                state: 0,
                slot: 0,
                button: 0,
                input: 0,
                changed: Vec::new(),
                carried: None,
            }),
        );
        let g = &game.server.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(
            g.ender_items[0].as_ref().map(|s| s.item.as_ref()),
            Some("minecraft:diamond")
        );
        assert!(g.carried.is_none(), "the stale click carries nothing");

        // The replacement heard no close sound or block event.
        let packets: Vec<_> = rx
            .try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                _ => Vec::new(),
            })
            .filter_map(|body| {
                let mut reader = Reader::new(&body);
                let id = reader.varint().ok()?;
                sunrise_protocol::packets::play::Clientbound::decode(id, reader.rest()).ok()
            })
            .collect();
        use sunrise_protocol::packets::play::Clientbound as C;
        assert!(
            !packets.iter().any(|p| matches!(
                p,
                C::BlockEvent(e) if e.pos == [pos.0, pos.1, pos.2]
            )),
            "the replacement gets no lid/opener event"
        );
        assert!(
            !packets.iter().any(|p| matches!(p, C::Sound(_))),
            "the replacement gets no close sound"
        );
    });
}
