//! A mod presses the vanilla gameplay mappings (`set_mapping_down`, `click_mapping`) on a server's
//! client: the client tick's key step acts on them with its checks and sends the packets the
//! player's own keys send.
use super::super::super::Screen;
use super::*;
use mod_api::prelude::ClientInputClientHost;
use mod_api::{ClientHost, InteractionKeyMappingTriggeredEvent, MappingError};

#[path = "input_burst_tests.rs"]
mod input_burst_tests;

/// The packets the client sends, and the hands the interaction hook hears.
struct Heard {
    packets: Arc<Mutex<Vec<String>>>,
    triggered: Arc<Mutex<Vec<Hand>>>,
    click_in_callback: Arc<std::sync::atomic::AtomicBool>,
}
#[mod_api::__subscriptions]
impl Hooks for Heard {
    fn client_packet_send(
        &self,
        _: &dyn ClientHost,
        e: &mut mod_api::ClientPacketSendEvent<'_>,
    ) -> Flow {
        self.packets.lock().unwrap().push(e.packet.id.to_string());
        Flow::Continue
    }
    fn interaction_key_mapping_triggered(
        &self,
        host: &mut dyn ClientHost,
        e: &mut InteractionKeyMappingTriggeredEvent,
    ) -> Flow {
        self.triggered.lock().unwrap().push(e.hand);
        if self
            .click_in_callback
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            host.click_mapping("key.attack").unwrap();
        }
        Flow::Continue
    }
}

/// A survival player on a server's client, playing with nothing open, the tick's first packets
/// (the held slot) sent: `test` gets the game, the packets sent since and the hands triggered.
fn with_pressing_game(test: impl FnOnce(&mut Game<'_>, &Mutex<Vec<String>>, &Mutex<Vec<Hand>>)) {
    with_callback_game(false, test);
}
fn with_callback_game(
    click: bool,
    test: impl FnOnce(&mut Game<'_>, &Mutex<Vec<String>>, &Mutex<Vec<Hand>>),
) {
    let packets = Arc::new(Mutex::new(Vec::new()));
    let triggered = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "presses",
        Box::new(Heard {
            packets: packets.clone(),
            triggered: triggered.clone(),
            click_in_callback: Arc::new(std::sync::atomic::AtomicBool::new(click)),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(true, |game| {
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            game.set_mode_id(0);
            game.title = None;
            game.screen = Screen::Playing;
            game.remote.as_mut().unwrap().loaded_sent = true;
            // The tick's first packet, the held slot (`MultiPlayerGameMode.tick`), then its keys.
            game.remote_send_carried();
            game.remote_handle_input();
            packets.lock().unwrap().clear();
            test(game, &packets, &triggered);
        })
    });
}

/// What was sent since the last call.
fn sent(packets: &Mutex<Vec<String>>) -> Vec<String> {
    std::mem::take(&mut *packets.lock().unwrap())
}

fn stone_under_the_crosshair(game: &mut Game<'_>) {
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(0, 64, 0, stone).unwrap();
    game.forced_hit = Some(hit());
}

/// Holding use with a bow drawn starts one use and repeats nothing while the bow is in use;
/// letting go releases it, as letting go of the key does.
#[test]
#[ignore = "daily"]
fn holding_use_draws_a_bow_once_and_letting_go_releases_it() {
    with_pressing_game(|game, packets, triggered| {
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:bow", 1));
        game.inventory[9] = Some(ItemStack::new("minecraft:arrow", 64));
        let sequence = game.remote.as_ref().unwrap().sequence;
        assert_eq!(game.set_mapping_down("key.use", true), Ok(()));
        assert!(game.right);
        assert_eq!(
            sent(packets),
            Vec::<String>::new(),
            "nothing before the tick"
        );
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:use_item"]);
        assert_eq!(game.remote.as_ref().unwrap().sequence - sequence, 1);
        assert!(game.using.is_some());
        assert_eq!(*triggered.lock().unwrap(), [Hand::Main]);
        for _ in 0..5 {
            game.use_delay = 0;
            game.remote_handle_input();
        }
        assert_eq!(
            sent(packets),
            Vec::<String>::new(),
            "no repeat while drawing"
        );
        assert_eq!(game.set_mapping_down("key.use", false), Ok(()));
        assert!(!game.right);
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:player_action"]);
        assert!(game.using.is_none());
        assert_eq!(*triggered.lock().unwrap(), [Hand::Main]);
    });
}

/// A click on a block in reach starts the break with a prediction sequence, then the punch; with
/// the key not held, the same tick's `continueAttack(false)` stops it, as a tap between two ticks
/// does. Held, the break goes on, punching each tick, and the release stops it.
#[test]
#[ignore = "daily"]
fn clicking_attack_on_a_block_starts_the_break_and_punches() {
    with_pressing_game(|game, packets, triggered| {
        stone_under_the_crosshair(game);
        let sequence = game.remote.as_ref().unwrap().sequence;
        assert_eq!(game.click_mapping("key.attack"), Ok(()));
        game.remote_handle_input();
        assert_eq!(
            sent(packets),
            [
                "minecraft:player_action",
                "minecraft:punch",
                "minecraft:player_action"
            ]
        );
        assert_eq!(game.remote.as_ref().unwrap().sequence - sequence, 1);
        assert_eq!(*triggered.lock().unwrap(), [Hand::Main]);

        let sequence = game.remote.as_ref().unwrap().sequence;
        assert_eq!(game.set_mapping_down("key.attack", true), Ok(()));
        assert_eq!(game.click_mapping("key.attack"), Ok(()));
        game.remote_handle_input();
        // The same tick's `continueAttack(true)` digs on and punches again, as a held press does.
        assert_eq!(
            sent(packets),
            [
                "minecraft:player_action",
                "minecraft:punch",
                "minecraft:punch"
            ]
        );
        assert_eq!(game.remote.as_ref().unwrap().sequence - sequence, 1);
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:punch"], "the held key digs on");
        assert_eq!(game.set_mapping_down("key.attack", false), Ok(()));
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:player_action"]);
    });
}

/// A click on nothing punches and sets the miss cooldown (not in creative); a second click inside
/// it does nothing.
#[test]
#[ignore = "daily"]
fn clicking_attack_on_nothing_punches_and_waits_out_the_miss_cooldown() {
    with_pressing_game(|game, packets, _| {
        game.forced_hit = None;
        assert_eq!(game.set_mapping_down("key.attack", true), Ok(()));
        assert_eq!(game.click_mapping("key.attack"), Ok(()));
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:punch"]);
        // 10, less the tick's own count down after `handleKeybinds`.
        assert_eq!(game.clicks.miss_time, 9);
        assert_eq!(game.click_mapping("key.attack"), Ok(()));
        game.remote_handle_input();
        assert_eq!(sent(packets), Vec::<String>::new(), "inside the cooldown");
        assert_eq!(game.clicks.miss_time, 8);

        game.clicks.miss_time = 0;
        game.set_mode_id(1);
        assert_eq!(game.click_mapping("key.attack"), Ok(()));
        game.remote_handle_input();
        assert_eq!(sent(packets), ["minecraft:punch"]);
        assert_eq!(game.clicks.miss_time, 0, "no cooldown in creative");
    });
}

/// Behind a screen a click is dropped and a press holds nothing, and a click queued before the
/// screen opened is not acted on.
#[test]
#[ignore = "daily"]
fn a_screen_drops_the_clicks_and_holds_nothing() {
    with_pressing_game(|game, packets, triggered| {
        stone_under_the_crosshair(game);
        game.screen = Screen::Inventory;
        assert_eq!(game.click_mapping("key.attack"), Ok(()));
        assert_eq!(game.set_mapping_down("key.attack", true), Ok(()));
        assert!(!game.left);
        game.remote_handle_input();
        game.screen = Screen::Playing;
        game.remote_handle_input();
        assert_eq!(sent(packets), Vec::<String>::new());

        assert_eq!(game.click_mapping("key.attack"), Ok(()));
        game.screen = Screen::Inventory;
        game.remote_handle_input();
        game.screen = Screen::Playing;
        game.remote_handle_input();
        assert_eq!(sent(packets), Vec::<String>::new());
        assert_eq!(*triggered.lock().unwrap(), Vec::<Hand>::new());
    });
}

/// A hotbar click selects its slot on the next tick, through the host as a mod calls it; the names
/// are refused as the contract lists, and nothing is pressed with no player in a world.
#[test]
#[ignore = "daily"]
fn the_host_presses_by_name_and_refuses_the_others() {
    with_pressing_game(|game, packets, _| {
        assert_eq!(
            ClientInputClientHost::click_mapping(game, "key.hotbar.3"),
            Ok(())
        );
        game.remote_handle_input();
        assert_eq!(game.selected, 2);
        assert_eq!(sent(packets), Vec::<String>::new());
        // `MultiPlayerGameMode.tick` tells the server first thing in the next tick.
        game.remote_send_carried();
        assert_eq!(sent(packets), ["minecraft:set_carried_item"]);
        assert_eq!(
            ClientInputClientHost::click_mapping(game, "key.screenshot"),
            Err(MappingError::NotGameplay {
                key: "key.screenshot".into()
            })
        );
        assert_eq!(
            ClientInputClientHost::set_mapping_down(game, "nope", true),
            Err(MappingError::Unknown { key: "nope".into() })
        );
        assert_eq!(
            ClientInputClientHost::set_mapping_down(game, "key.pickItem", true),
            Err(MappingError::ClickOnly {
                key: "key.pickItem".into()
            })
        );
        assert_eq!(
            ClientInputClientHost::click_mapping(game, "key.pickItem"),
            Ok(())
        );
        game.player = None;
        assert_eq!(
            ClientInputClientHost::set_mapping_down(game, "key.forward", true),
            Err(MappingError::NoWorld)
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_callback_click_survives_until_the_next_tick() {
    with_callback_game(true, |game, _, triggered| {
        stone_under_the_crosshair(game);
        game.click_mapping("key.attack").unwrap();
        game.remote_handle_input();
        assert_eq!(triggered.lock().unwrap().len(), 1);
        // The callback's second click is not processed recursively in this tick.
        game.remote_handle_input();
        assert_eq!(triggered.lock().unwrap().len(), 2);
        game.remote_handle_input();
        assert_eq!(triggered.lock().unwrap().len(), 2);
    });
}

#[test]
#[ignore = "daily"]
fn opening_a_screen_releases_mod_holds() {
    with_pressing_game(|game, _, _| {
        use sunrise_client::held::Source;
        use winit::event::MouseButton;
        use winit::keyboard::{KeyCode, ModifiersState};

        game.mapped_key_event(KeyCode::KeyW, true, false).unwrap();
        assert!(game.mapping_state("key.forward").unwrap().0);
        game.set_mapping_down("key.forward", false).unwrap();
        assert!(!game.mapping_state("key.forward").unwrap().0);
        game.set_mapping_down("key.forward", true).unwrap();
        game.mapped_key_event(KeyCode::KeyW, false, false).unwrap();
        assert!(!game.mapping_state("key.forward").unwrap().0);
        game.set_mapping_down("key.forward", true).unwrap();
        game.set_mapping_down("key.attack", true).unwrap();
        game.set_mapping_down("key.use", true).unwrap();
        assert!(game.left && game.right);
        game.physical_buttons
            .extend([MouseButton::Left, MouseButton::Right]);
        game.screen = Screen::Inventory;
        game.build_screen();
        // No redraw/reconciliation before the screen closes. Controller releases
        // forwarded for cleanup cannot give the hold back to the mouse in a screen.
        for button in [MouseButton::Left, MouseButton::Right] {
            game.mouse_mapping(button, false, Source::Pad).unwrap();
        }
        game.screen = Screen::Playing;
        game.build_screen();
        assert!(!game.left && !game.right);
        assert!(!game.mapping_state("key.forward").unwrap().0);

        // ModifiersChanged can contain an unrelated change, or be the OS's only
        // report of a real release. Only the recovered sneak release takes over.
        game.settings.toggle_crouch = false;
        game.physical_keys.insert(KeyCode::ShiftLeft);
        game.update_modifiers(ModifiersState::SHIFT | ModifiersState::ALT);
        game.mapped_key_event(KeyCode::ShiftLeft, true, false)
            .unwrap();
        game.set_mapping_down("key.sneak", true).unwrap();
        game.update_modifiers(ModifiersState::SHIFT);
        game.heal_hold_latches();
        assert!(game.mapping_state("key.sneak").unwrap().0);
        game.update_modifiers(ModifiersState::empty());
        assert!(!game.mapping_state("key.sneak").unwrap().0);
    });
}

/// Q in creative mode: `MultiPlayerGameMode.dropItem` sends the drop and nothing else, so the
/// end of the tick sends no creative slot edit for the stack it left (only the creative screen
/// sends those).
#[test]
#[ignore = "daily"]
fn a_creative_drop_sends_no_creative_slot() {
    with_pressing_game(|game, packets, _| {
        game.set_mode_id(1);
        game.forced_hit = None;
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:stone", 16));
        // The setup's stack is the server's as well.
        game.remote_synced();
        game.key_pressed(
            winit::keyboard::KeyCode::KeyQ,
            winit::keyboard::KeyCode::KeyQ,
        )
        .unwrap();
        game.remote_handle_input();
        game.remote_send_state(0);
        let sent = sent(packets);
        assert!(
            sent.iter().any(|p| p == "minecraft:player_action"),
            "{sent:?}"
        );
        assert!(
            !sent.iter().any(|p| p == "minecraft:set_creative_mode_slot"),
            "{sent:?}"
        );
        assert_eq!(
            game.inventory[game.selected].as_ref().map(|s| s.count),
            Some(15)
        );
    });
}

/// A creative toolbar load and Q in one tick (`handleKeybinds`: the number keys' loop, then
/// key.drop): the load sends the new stack at once (`handleCreativeModeItemAdd`), so the drop
/// that follows drops what the server now holds, and the end of the tick sends nothing more.
#[test]
#[ignore = "daily"]
fn a_toolbar_load_goes_out_before_a_drop_in_its_tick() {
    with_pressing_game(|game, packets, _| {
        game.set_mode_id(1);
        game.forced_hit = None;
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:stone", 16));
        game.remote_synced();
        // handleHotbarLoadOrSave puts the saved stack in the slot and sends it.
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:dirt", 16));
        game.remote_creative_slot_add(game.selected);
        game.key_pressed(
            winit::keyboard::KeyCode::KeyQ,
            winit::keyboard::KeyCode::KeyQ,
        )
        .unwrap();
        game.remote_handle_input();
        game.remote_send_state(0);
        let sent = sent(packets);
        let at = |id: &str| sent.iter().position(|p| p == id);
        assert_eq!(
            sent.iter()
                .filter(|p| *p == "minecraft:set_creative_mode_slot")
                .count(),
            1,
            "{sent:?}"
        );
        assert!(
            at("minecraft:set_creative_mode_slot") < at("minecraft:player_action"),
            "{sent:?}"
        );
        assert_eq!(
            game.inventory[game.selected]
                .as_ref()
                .map(|s| (s.item.as_ref(), s.count)),
            Some(("minecraft:dirt", 15))
        );
    });
}

/// `MultiPlayerGameMode.tick` comes first in `Minecraft.tick`, before the GUI ticks its
/// screens: a slot chosen since the last tick goes out ahead of what a screen sends as it
/// closes in that tick (a sign editor whose sign is gone, `AbstractSignEditScreen.removed`).
#[test]
#[ignore = "daily"]
fn the_held_slot_goes_out_before_a_closing_screens_packet() {
    with_pressing_game(|game, packets, _| {
        game.open_sign_editor((0, 64, 40), true);
        sent(packets);
        game.selected = (game.selected + 1) % 9;
        game.tick_remote_level().unwrap();
        let sent = sent(packets);
        let at = |id: &str| sent.iter().position(|p| p == id);
        assert!(at("minecraft:sign_update").is_some(), "{sent:?}");
        assert!(at("minecraft:set_carried_item").is_some(), "{sent:?}");
        assert!(
            at("minecraft:set_carried_item") < at("minecraft:sign_update"),
            "{sent:?}"
        );
    });
}
