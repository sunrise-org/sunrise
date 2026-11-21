//! A mod's registered mapping read through the client host, fed by the control socket as by the
//! window: a tap is one click and one `input_key` press, each repeat one more click, a screen or
//! overlay opening drops them at once, a binding's modifier must be held, a rebinding moves the
//! mapping, and a mouse press counts for a mapping on its button.
use super::event_loop::EventLoopCtl;
use super::{Game, Screen};
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use mod_api::prelude::ClientInputClientHost;
use mod_api::*;
use std::sync::Mutex;
use winit::event_loop::ControlFlow;

struct NoLoop;
impl EventLoopCtl for NoLoop {
    fn exit(&self) {}
    fn set_control_flow(&self, _: ControlFlow) {}
}

/// The `input_key` events heard, as `key action`.
static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Registers `zoom` on Z, `boost` on Shift + Z and `grab` on the left mouse button, and hears
/// `input_key`.
struct Keys;

#[mod_api::__subscriptions]
impl Hooks for Keys {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_key_mapping(KeyRegistration::new("zoom", "key.keyboard.z"));
        r.register_key_mapping(
            KeyRegistration::new("boost", "key.keyboard.z").modifiers(ClientModifiers::new(1)),
        );
        r.register_key_mapping(KeyRegistration::new("grab", "key.mouse.left"));
    }
    fn input_key(&self, _: &mut dyn ClientHost, event: &mut InputKeyEvent<'_>) {
        HEARD
            .lock()
            .unwrap()
            .push(format!("{} {:?}", event.key, event.action));
    }
}

fn down(game: &Game<'_>, id: &str) -> bool {
    ClientInputClientHost::is_down(game, "keys", id)
}
fn click(game: &mut Game<'_>, id: &str) -> bool {
    ClientInputClientHost::consume_click(game, "keys", id)
}
fn line(game: &mut Game<'_>, line: &str) {
    game.replay_line(line, &NoLoop);
}
fn heard() -> Vec<String> {
    std::mem::take(&mut *HEARD.lock().unwrap())
}

#[test]
#[ignore = "daily"]
fn a_control_socket_key_counts_as_a_window_key() {
    if run_fresh_cases(
        "game::native_key_mappings_tests::a_control_socket_key_counts_as_a_window_key",
        "SUNRISE_MOD_KEYS_CHILD",
        &["game"],
    ) {
        return;
    }
    let mods: Mods = vec![ModEntry::new("keys", Box::new(Keys))].into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    with_client_game(|game| {
        game.apply_settings();
        game.set_screen(Screen::Playing);
        heard();

        // A tap: one click, consumed once; held only while down; `input_key` hears one press
        // and one release.
        line(game, "key z tap");
        assert!(!down(game, "zoom"));
        assert!(click(game, "zoom"));
        assert!(!click(game, "zoom"));
        assert_eq!(heard(), ["key.keyboard.z Press", "key.keyboard.z Release"]);
        // The window's tap takes the same route.
        let raw = sunrise_client::keybinds::raw_key_code(winit::keyboard::PhysicalKey::Code(
            winit::keyboard::KeyCode::KeyZ,
        ));
        let z = Some("key.keyboard.z");
        game.key_input(z, z, raw, KeyAction::Press, 0);
        assert!(down(game, "zoom"));
        game.key_input(z, z, raw, KeyAction::Release, 0);
        assert!(!down(game, "zoom"));
        assert!(click(game, "zoom"));
        assert!(!click(game, "zoom"));
        assert_eq!(heard(), ["key.keyboard.z Press", "key.keyboard.z Release"]);

        // Held with two repeats: three clicks.
        line(game, "key z down");
        line(game, "key z repeat");
        line(game, "key z repeat");
        assert!(down(game, "zoom"));
        line(game, "key z up");
        assert!(!down(game, "zoom"));
        let clicks: Vec<bool> = (0..4).map(|_| click(game, "zoom")).collect();
        assert_eq!(clicks, [true, true, true, false]);
        assert_eq!(heard().len(), 4);

        // A screen opening drops the clicks at once, even closed again before anyone looks.
        line(game, "key z tap");
        game.set_screen(Screen::Inventory);
        assert!(!click(game, "zoom"));
        line(game, "key z tap");
        game.set_screen(Screen::Playing);
        assert!(!click(game, "zoom"), "pressed behind the screen");
        line(game, "key z down");
        game.set_screen(Screen::Inventory);
        game.set_screen(Screen::Playing);
        assert!(!down(game, "zoom"));
        assert!(!click(game, "zoom"));
        line(game, "key z up");
        // An overlay too.
        line(game, "key z tap");
        game.open_screen_overlay("link_prompt", |game| {
            game.link_prompt = Some("https://example.com".into());
        });
        game.link_prompt = None;
        assert!(!click(game, "zoom"));

        // Shift + Z counts only with shift held; Z alone counts with it too.
        line(game, "key z tap");
        assert!(!click(game, "boost"));
        line(game, "key shift down");
        line(game, "key z tap");
        line(game, "key shift up");
        assert!(click(game, "boost"));
        assert!(!click(game, "boost"));
        let clicks: Vec<bool> = (0..3).map(|_| click(game, "zoom")).collect();
        assert_eq!(clicks, [true, true, false]);

        // Rebound in Controls to X: Z no longer counts, X does.
        game.settings
            .mods
            .set_binding("keys", "zoom", "key.keyboard.x", 0);
        game.apply_settings();
        line(game, "key z tap");
        line(game, "key x tap");
        let clicks: Vec<bool> = (0..2).map(|_| click(game, "zoom")).collect();
        assert_eq!(clicks, [true, false]);

        // A mouse press counts for a mapping on its button.
        line(game, "mouse left click");
        assert!(click(game, "grab"));
        assert!(!click(game, "grab"));

        // Another mod's id, or an unregistered one, has nothing.
        assert!(!ClientInputClientHost::consume_click(game, "other", "zoom"));
        assert!(!click(game, "missing"));
    });
}

/// A mod rebinds its own mapping as Controls does: the new key counts from the next input, the
/// binding is saved, a reset gives the default back, and each refusal changes nothing.
#[test]
#[ignore = "daily"]
fn a_mod_rebinds_and_resets_its_own_mapping() {
    if run_fresh_cases(
        "game::native_key_mappings_tests::a_mod_rebinds_and_resets_its_own_mapping",
        "SUNRISE_MOD_KEYS_REBIND_CHILD",
        &["game"],
    ) {
        return;
    }
    let dir = std::env::temp_dir().join(format!("sunrise-mod-keys-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // The settings save to options.txt in the player folder and sunrise-mods.txt in the game
    // folder (the working directory): both are this child process's temporary folder.
    std::env::set_current_dir(&dir).unwrap();
    sunrise_client::logging::set_player_dir(dir.clone());
    // The saved mod list, as the next start reads it: mod `keys`'s binding of `id`.
    let saved = |id: &str| {
        sunrise_client::mod_list::ModList::load(&dir).binding("keys", id, ("default", 99))
    };
    let mods: Mods = vec![ModEntry::new("keys", Box::new(Keys))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        with_client_game(|game| {
            game.apply_settings();
            game.set_screen(Screen::Playing);
            let key = |game: &Game<'_>, id: &str| ClientInputClientHost::mod_key(game, "keys", id);
            let set = |game: &mut Game<'_>, id: &str, key: &str, bits: u32| {
                ClientInputClientHost::set_mod_key(
                    game,
                    "keys",
                    id,
                    key,
                    ClientModifiers::new(bits),
                )
            };

            // Rebound to X with control: Z no longer counts; X counts only with control held.
            assert_eq!(set(game, "zoom", "key.keyboard.x", 2), Ok(()));
            assert_eq!(
                key(game, "zoom"),
                Some(("key.keyboard.x".into(), ClientModifiers::new(2)))
            );
            line(game, "key z tap");
            line(game, "key x tap");
            assert!(!click(game, "zoom"));
            line(game, "key control down");
            line(game, "key x tap");
            line(game, "key control up");
            assert!(click(game, "zoom"));
            // Saved as Controls saves it: the next start reads exactly this binding.
            assert_eq!(saved("zoom"), ("key.keyboard.x".to_owned(), 2));

            // A modifier key bound on its own is the key, not a modifier held with itself.
            assert_eq!(set(game, "boost", "key.keyboard.left.shift", 1), Ok(()));
            assert_eq!(
                key(game, "boost"),
                Some(("key.keyboard.left.shift".into(), ClientModifiers::new(0)))
            );
            assert_eq!(saved("boost"), ("key.keyboard.left.shift".to_owned(), 0));

            // Reset: the declared default is back, Z counts again, and the saved list holds no
            // binding of it any more (the default answers).
            assert_eq!(
                ClientInputClientHost::reset_mod_key(game, "keys", "zoom"),
                Ok(())
            );
            assert_eq!(
                key(game, "zoom"),
                Some(("key.keyboard.z".into(), ClientModifiers::new(0)))
            );
            line(game, "key z tap");
            assert!(click(game, "zoom"));
            assert_eq!(saved("zoom"), ("default".to_owned(), 99));

            // Refusals change nothing, in the game or in the saved list.
            let before = (key(game, "zoom"), key(game, "boost"));
            let file = || std::fs::read_to_string(dir.join("sunrise-mods.txt")).unwrap();
            let file_before = file();
            assert_eq!(
                set(game, "missing", "key.keyboard.x", 0),
                Err(ModKeyRefusal::NotRegistered)
            );
            assert_eq!(
                set(game, "zoom", "key.keyboard.nope", 0),
                Err(ModKeyRefusal::UnknownKey)
            );
            assert_eq!(
                set(game, "zoom", "key.keyboard.x", 8),
                Err(ModKeyRefusal::UnknownModifiers)
            );
            assert_eq!(
                ClientInputClientHost::reset_mod_key(game, "other", "zoom"),
                Err(ModKeyRefusal::NotRegistered)
            );
            // Inside another native mod's hook, neither call reaches `keys`'s mappings.
            {
                let _other = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "other",
                        hook: "screen_click",
                        part: None,
                    },
                );
                assert_eq!(
                    set(game, "zoom", "key.keyboard.x", 0),
                    Err(ModKeyRefusal::NotOwner)
                );
                assert_eq!(
                    ClientInputClientHost::reset_mod_key(game, "keys", "boost"),
                    Err(ModKeyRefusal::NotOwner)
                );
            }
            assert_eq!((key(game, "zoom"), key(game, "boost")), before);
            assert_eq!(file(), file_before);
            // Inside `keys`'s own hook, the same call goes through.
            {
                let _own = mod_platform::content::crash::enter_native_mod(
                    mod_api::content::crash::ModCall {
                        id: "keys",
                        hook: "screen_click",
                        part: None,
                    },
                );
                assert_eq!(set(game, "zoom", "key.keyboard.y", 0), Ok(()));
            }
            assert_eq!(saved("zoom"), ("key.keyboard.y".to_owned(), 0));

            // A mouse button and "no key" are bindings too.
            assert_eq!(set(game, "grab", "key.mouse.right", 0), Ok(()));
            assert_eq!(saved("grab"), ("key.mouse.right".to_owned(), 0));
            assert_eq!(set(game, "grab", "key.keyboard.unknown", 0), Ok(()));
            assert_eq!(saved("grab"), ("key.keyboard.unknown".to_owned(), 0));
        });
    });
    let _ = std::fs::remove_dir_all(&dir);
}
