use super::*;
use std::sync::OnceLock;
use sunrise_client::recipe_book::{BookFlags, settings_codec, settings_sync::Message};

static KEY: OnceLock<mod_api::RecipeBookTypeKey> = OnceLock::new();
struct BookMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for BookMod {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        registrar.register_recipe_type("geometry:crafting");
        registrar.register_raw_recipe_serializer("geometry:crafting", "geometry:crafting");
        let key = registrar.recipe_book_type("geometry:book").unwrap();
        assert_eq!(KEY.get_or_init(|| key.clone()), &key);
    }
    fn recipe_book_categories(
        &self,
        _: mod_api::HostSide,
        event: &mut mod_api::RecipeBookCategoriesEvent<'_>,
    ) {
        event.categories.push(mod_api::RecipeBookCategory {
            id: "geometry:initial".into(),
            recipe_type: "geometry:crafting".into(),
            icon: "minecraft:stone".into(),
            secondary_icon: None,
            target_menu: "geometry:menu".into(),
            book: KEY.get().unwrap().clone(),
            search_group: "geometry:search".into(),
            displays: Vec::new(),
        });
    }
}
fn receive(game: &mut Game<'_>, message: Message) {
    game.remote_book_settings(&settings_codec::encode(&message).unwrap());
}
struct BoundBookMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for BoundBookMod {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let key = registrar.recipe_book_type("bound:book").unwrap();
        registrar
            .menu_recipe_book(
                "bound:menu",
                &key,
                "bound:recipe",
                mod_api::RecipeBookLayout::Crafting {
                    width: 1,
                    height: 1,
                    slots: vec![0],
                    result: 1,
                },
                mod_api::Screen {
                    width: 176.0,
                    height: 166.0,
                    slots: vec![
                        Some(mod_api::ScreenSlot {
                            position: [8.0, 18.0],
                            ..Default::default()
                        }),
                        Some(mod_api::ScreenSlot {
                            position: [80.0, 18.0],
                            ..Default::default()
                        }),
                    ],
                    recipe_book: Some(key.clone()),
                    recipe_book_toggle: Some(mod_api::RecipeBookToggle {
                        x: 153.0,
                        y: 60.0,
                        from: mod_api::ToggleOrigin::Menu,
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
    }
}
#[test]
#[ignore = "daily"]
fn entering_inventory_requests_catalogue_for_preinstalled_bound_screen() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("bound", Box::new(BoundBookMod))].into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            mod_api::__private::resolve_menus(["bound:menu"]).unwrap();
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "Bound").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.custom_books = true;
            remote.joined = true;
            remote.menu.set(Some((1, "bound:menu".into())));
            game.screen = Screen::Playing;
            game.container = Some((i32::MIN, 0, 0));
            game.mod_screen = Some(OpenNativeScreen {
                slot_moves: Default::default(),
                layout_revision: 4,
                key: (i32::MIN, 0, 0),
                window: 1,
                menu: "bound:menu".into(),
                block: Some("bound:block".into()),
                mod_type: None,
                data: Vec::new(),
                extra: None,
                screen: mod_api::RecipeBookLayout::registered_screen("bound:menu").unwrap(),
            });
            // Negotiation completed before the user opened the machine. As in
            // remote_open_screen, the declaration exists before Inventory is shown.
            use sunrise_client::recipe_book::placement_sync;
            game.native_net.placement.client.hello(6);
            assert!(
                game.native_net
                    .placement
                    .client
                    .ready(6, placement_sync::schema())
            );
            game.ensure_mod_book_catalog();
            assert!(game.native_net.placement.client.window().is_none());
            game.set_screen(Screen::Inventory);
            let window = game
                .native_net
                .placement
                .client
                .window()
                .expect("catalogue requested on opening");
            assert_eq!(window.menu.as_str(), "bound:menu");
            assert_eq!(window.nonce, 4);
            assert!(game.native_net.placement.deadline.is_some());
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_registered_recipe_book_screen_opens_for_a_blockless_menu() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("bound", Box::new(BoundBookMod))].into();
    sunrise_client::containers::with_mod_menu_types(
        [(
            "bound:menu".into(),
            "bound".into(),
            3,
            0,
            vec![1],
            vec![0],
            None,
            None,
            true,
            None,
        )],
        || {
            mod_platform::testing::with_mods(mods, || {
                crate::game::native_entity_refs::tests::with_client_game(|game| {
                    mod_api::__private::resolve_menus(["bound:menu"]).unwrap();
                    game.open_native_screen(
                        (i32::MIN, 0, 0),
                        None,
                        7,
                        "bound:menu",
                        Some(sunrise_protocol::menu_extra::Message {
                            window: 7,
                            pos: None,
                            data: Vec::new(),
                        }),
                    );
                    let opened = game
                        .mod_screen
                        .as_ref()
                        .expect("blockless book screen opens");
                    assert_eq!(opened.window, 7);
                    assert_eq!(opened.menu, "bound:menu");
                    assert_eq!(opened.block, None);
                    assert_eq!(
                        opened
                            .screen
                            .recipe_book
                            .as_ref()
                            .map(mod_api::RecipeBookTypeKey::as_str),
                        Some("bound:book")
                    );
                    assert_eq!(game.native_hidden_slots("bound:menu", None, 3), vec![2]);
                });
            });
        },
    );
}

const STONE: &str = "minecraft:stone";
const MENU: &str = "minecraft:generic_9x1";
fn geometry(game: &Game<'_>) -> ((f32, f32), Vec<(usize, [f32; 4])>) {
    let batch = sunrise_client::hud::HudBatch::new([1280, 720], game.settings.gui_scale_inputs());
    let layout = game.built_container(&batch).expect("cached container");
    (layout.origin, layout.slots.clone())
}
#[test]
#[ignore = "daily"]
fn authoritative_custom_book_rebuilds_cached_geometry_without_observe_screen() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("geometry", Box::new(BookMod))].into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let key = KEY.get().unwrap().clone();
            // The lightweight Game fixture does not run the launcher/content
            // bootstrap that publishes the negotiated menu registry.
            mod_api::__private::resolve_menus(["geometry:menu"]).unwrap();
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "Geometry").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.joined = true;
            remote.menu.set(Some((1, "geometry:menu".into())));
            // A registered book alone must not authorize the private protocol.
            receive(game, Message::Ready);
            assert!(!game.native_net.book_ready);
            game.remote.as_mut().unwrap().custom_books = true;
            game.settings.gui_scale = 1;
            // The fixture has no renderer; layout still needs the same viewport
            // used by geometry() to evaluate toggle gutters and book placement.
            game.recipe_ui.test_viewport = Some([1280, 720]);
            game.screen = Screen::Inventory;
            game.title = None;
            game.container = Some((i32::MIN, 0, 0));
            game.mod_screen = Some(OpenNativeScreen {
                slot_moves: Default::default(),
                layout_revision: 0,
                key: (i32::MIN, 0, 0),
                window: 1,
                menu: "geometry:menu".into(),
                block: Some("minecraft:stone".into()),
                mod_type: None,
                data: Vec::new(),
                extra: None,
                screen: mod_api::Screen {
                    width: 176.0,
                    height: 166.0,
                    slots: vec![Some(mod_api::ScreenSlot {
                        position: [8.0, 18.0],
                        ..Default::default()
                    })],
                    inventory_y: 84.0,
                    inventory_x: 8.0,
                    title_label: None,
                    inventory_label: None,
                    own_labels: false,
                    own_background: false,
                    recipe_book: Some(key.clone()),
                    recipe_book_toggle: None,
                    recipe_book_big_result: false,
                },
            });
            // A snapshot may precede the first draw. The build itself must
            // collect declarations, not wait for the render refresh path.
            let recipes = sunrise_client::crafting::Recipes::from_sources(
                &game.options.resources,
                game.items(),
                &sunrise_world::loot::PackOverlay::NONE,
                vec![(
                    "geometry:recipe".into(),
                    serde_json::json!({"type": "geometry:crafting"}),
                )],
                game.res.core.item_data.context(),
            )
            .unwrap();
            game.recipe_tables.install_test_recipes(recipes);
            game.recipe_tables.books = None;
            receive(game, Message::Ready);
            receive(
                game,
                Message::Snapshot {
                    epoch: 6,
                    revision: 0,
                    entries: vec![(
                        key.as_str().into(),
                        BookFlags {
                            open: true,
                            filtering: false,
                        },
                    )],
                },
            );
            game.build_screen();
            if let Err(error) = game
                .recipe_tables
                .books
                .as_ref()
                .expect("build collected declarations")
            {
                panic!("declaration refused: {error:?}");
            }
            assert_eq!(geometry(game).0, (629.0, 277.0));
            game.reset_book_view();
            receive(game, Message::Ready);
            receive(
                game,
                Message::Snapshot {
                    epoch: 7,
                    revision: 0,
                    entries: vec![(
                        key.as_str().into(),
                        BookFlags {
                            open: false,
                            filtering: false,
                        },
                    )],
                },
            );
            game.build_screen();
            let closed = geometry(game);
            assert_eq!(closed.0, (552.0, 277.0));
            assert!(closed.1.contains(&(300, [559.0, 294.0, 18.0, 18.0])));
            let saved = game.level.recipe_book.custom.clone();
            assert!(game.request_custom_book_settings(
                &key,
                mod_api::RecipeBookSettings {
                    open: true,
                    filtering: true
                }
            ));
            assert_eq!(geometry(game), closed);
            assert_eq!(game.level.recipe_book.custom, saved);
            receive(
                game,
                Message::Update {
                    epoch: 7,
                    revision: 1,
                    key: key.as_str().into(),
                    flags: BookFlags {
                        open: true,
                        filtering: true,
                    },
                },
            );
            let opened = geometry(game);
            assert_eq!(opened.0, (629.0, 277.0));
            assert!(opened.1.contains(&(300, [636.0, 294.0, 18.0, 18.0])));
            assert_ne!(opened.1, closed.1);
            assert_eq!(opened.1.len(), closed.1.len());
            let shift = opened.0.0 - closed.0.0;
            assert_eq!(opened.0.1, closed.0.1);
            for ((id, before), (after_id, after)) in closed.1.iter().zip(&opened.1) {
                assert_eq!(id, after_id);
                assert_eq!(*after, [before[0] + shift, before[1], before[2], before[3]]);
            }
            receive(
                game,
                Message::Update {
                    epoch: 7,
                    revision: 2,
                    key: key.as_str().into(),
                    flags: BookFlags {
                        open: false,
                        filtering: false,
                    },
                },
            );
            assert_eq!(geometry(game), closed);
            receive(
                game,
                Message::Update {
                    epoch: 7,
                    revision: 3,
                    key: key.as_str().into(),
                    flags: BookFlags {
                        open: true,
                        filtering: false,
                    },
                },
            );
            assert_eq!(geometry(game), opened);
            receive(game, Message::Busy);
            assert_eq!(geometry(game), closed);
            receive(
                game,
                Message::Snapshot {
                    epoch: 7,
                    revision: 4,
                    entries: vec![(
                        key.as_str().into(),
                        BookFlags {
                            open: true,
                            filtering: false,
                        },
                    )],
                },
            );
            assert_eq!(geometry(game), opened);
            game.reset_book_view();
            assert_eq!(geometry(game), closed);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_moved_slot_takes_its_place_for_drawing_and_input_from_the_next_frame() {
    let mods: mod_api::Mods = Vec::<mod_api::ModEntry>::new().into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.settings.gui_scale = 1;
            game.recipe_ui.test_viewport = Some([1280, 720]);
            game.screen = Screen::Inventory;
            game.title = None;
            game.container = Some((i32::MIN, 0, 0));
            game.mod_screen = Some(OpenNativeScreen {
                slot_moves: Default::default(),
                layout_revision: 0,
                key: (i32::MIN, 0, 0),
                window: 1,
                menu: "minecraft:generic_9x1".into(),
                block: Some("minecraft:stone".into()),
                mod_type: None,
                data: Vec::new(),
                extra: None,
                screen: mod_api::Screen {
                    width: 176.0,
                    height: 166.0,
                    slots: vec![
                        Some(mod_api::ScreenSlot {
                            position: [8.0, 18.0],
                            ..Default::default()
                        }),
                        Some(mod_api::ScreenSlot {
                            position: [26.0, 18.0],
                            ..Default::default()
                        }),
                        None,
                    ],
                    inventory_y: 84.0,
                    inventory_x: 8.0,
                    ..Default::default()
                },
            });
            game.build_screen();
            let before = geometry(game);
            // Origin (552, 277): slot 0's 18 by 18 box starts one pixel up and left of it.
            assert_eq!(before.0, (552.0, 277.0));
            assert!(before.1.contains(&(300, [559.0, 294.0, 18.0, 18.0])));
            let batch =
                sunrise_client::hud::HudBatch::new([1280, 720], game.settings.gui_scale_inputs());
            assert_eq!(game.slot_at(&batch, 600.0, 340.0), None);

            // A render hook moves it after this frame's build: drawing and
            // hit-testing both still use the old place this frame.
            assert!(game.set_screen_slot(MENU, 0, Some([48.0, 60.0])).is_ok());
            assert_eq!(game.slot_position(300), (8.0, 18.0));
            assert_eq!(game.slot_at(&batch, 600.0, 340.0), None);
            assert_eq!(game.slot_at(&batch, 560.0, 295.0), Some(300));
            assert_eq!(geometry(game), before);
            // The next frame's build takes it for both.
            game.build_screen();
            assert_eq!(game.slot_position(300), (48.0, 60.0));
            let after = geometry(game);
            assert!(after.1.contains(&(300, [599.0, 336.0, 18.0, 18.0])));
            // The 18 by 18 box holds the pointer at its first pixel and not past its last.
            assert_eq!(game.slot_at(&batch, 599.0, 336.0), Some(300));
            assert_eq!(game.slot_at(&batch, 616.0, 353.0), Some(300));
            assert_eq!(game.slot_at(&batch, 617.0, 340.0), None);
            assert_ne!(game.slot_at(&batch, 560.0, 295.0), Some(300));
            assert_eq!(game.slot_position(301), (26.0, 18.0));

            // The controller's slot targets read the same committed position.
            let pad: Vec<[f32; 4]> = game
                .pad_slot_targets(&batch)
                .iter()
                .map(|t| t.rect)
                .collect();
            assert!(pad.contains(&[600.0, 337.0, 16.0, 16.0]));
            assert!(!pad.contains(&[560.0, 295.0, 16.0, 16.0]));

            // Repeating the position, or the declared one, rebuilds nothing.
            let builds = crate::game::container_layout::LAYOUT_CONSTRUCTIONS.with(|c| c.get());
            assert!(game.set_screen_slot(MENU, 0, Some([48.0, 60.0])).is_ok());
            assert!(game.set_screen_slot(MENU, 1, Some([26.0, 18.0])).is_ok());
            game.build_screen();
            assert_eq!(
                crate::game::container_layout::LAYOUT_CONSTRUCTIONS.with(|c| c.get()),
                builds
            );
            assert!(!game.set_screen_slot(MENU, 2, Some([1.0, 1.0])).is_ok());
            assert!(!game.set_screen_slot(MENU, 9, Some([1.0, 1.0])).is_ok());
            assert_eq!(geometry(game), after);

            // Overlaps resolve by menu index: the container slot over the hotbar's
            // first slot (8, 142) and over the main inventory's first (8, 84).
            assert!(game.set_screen_slot(MENU, 0, Some([8.0, 142.0])).is_ok());
            assert!(game.set_screen_slot(MENU, 1, Some([8.0, 84.0])).is_ok());
            game.build_screen();
            assert_eq!(game.slot_at(&batch, 561.0, 420.0), Some(300));
            assert_eq!(game.slot_at(&batch, 561.0, 362.0), Some(301));

            // A slot beyond the image takes the click instead of making it an outside click.
            assert!(game.set_screen_slot(MENU, 0, Some([200.0, 20.0])).is_ok());
            game.build_screen();
            game.cursor = (555.0 + 200.0, 277.0 + 22.0);
            assert_eq!(game.target_at(&batch), (Some(300), false));
            game.cursor = (555.0 + 230.0, 277.0 + 22.0);
            assert_eq!(game.target_at(&batch), (None, true));
        });
    });
}

/// A hidden slot keeps its place this frame, then leaves drawing, listing and hit-testing from
/// the next build, and comes back where the mod puts it. Each refusal is named and changes
/// nothing; a slot hidden when the screen opened is never shown.
#[test]
#[ignore = "daily"]
fn a_hidden_slot_leaves_the_screen_and_comes_back_where_put() {
    use mod_api::ScreenSlotRefusal::*;
    use mod_api::prelude::ScreensClientHost;
    let mods: mod_api::Mods = Vec::<mod_api::ModEntry>::new().into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.settings.gui_scale = 1;
            game.recipe_ui.test_viewport = Some([1280, 720]);
            game.screen = Screen::Inventory;
            game.title = None;
            game.container = Some((i32::MIN, 0, 0));
            game.mod_screen = Some(OpenNativeScreen {
                slot_moves: Default::default(),
                layout_revision: 0,
                key: (i32::MIN, 0, 0),
                window: 1,
                menu: MENU.into(),
                block: Some(STONE.into()),
                mod_type: None,
                data: Vec::new(),
                extra: None,
                screen: mod_api::Screen {
                    width: 176.0,
                    height: 166.0,
                    slots: vec![
                        Some(mod_api::ScreenSlot {
                            position: [8.0, 18.0],
                            ..Default::default()
                        }),
                        Some(mod_api::ScreenSlot {
                            position: [26.0, 18.0],
                            ..Default::default()
                        }),
                        None,
                    ],
                    inventory_y: 84.0,
                    inventory_x: 8.0,
                    ..Default::default()
                },
            });
            game.build_screen();
            let batch =
                sunrise_client::hud::HudBatch::new([1280, 720], game.settings.gui_scale_inputs());
            assert_eq!(game.slot_at(&batch, 560.0, 295.0), Some(300));

            // Hidden by a render hook: still there this frame, gone from the next build.
            assert_eq!(game.set_screen_slot(MENU, 0, None), Ok(()));
            assert_eq!(ScreensClientHost::screen_slot(game, MENU, 0), None);
            assert_eq!(game.slot_at(&batch, 560.0, 295.0), Some(300));
            game.build_screen();
            assert_eq!(game.slot_at(&batch, 560.0, 295.0), None);
            assert!(!geometry(game).1.iter().any(|(id, _)| *id == 300));
            assert_eq!(game.native_screen_slots(), Some(vec![301]));
            // The 36 player slots and the remaining visible container slot.
            assert_eq!(game.slot_count(), 37);
            assert_eq!(game.native_screen_slot(0), None);

            // Shown again where the mod puts it.
            assert_eq!(game.set_screen_slot(MENU, 0, Some([48.0, 60.0])), Ok(()));
            assert_eq!(game.screen_slot(MENU, 0), Some([48.0, 60.0]));
            game.build_screen();
            assert_eq!(game.slot_at(&batch, 600.0, 340.0), Some(300));
            assert_eq!(game.native_screen_slots(), Some(vec![300, 301]));
            assert_eq!(game.screen_slot(MENU, 1), Some([26.0, 18.0]));

            // Refusals, each by name, and nothing changes.
            let before = geometry(game);
            assert_eq!(
                game.set_screen_slot(MENU, 2, Some([1.0, 1.0])),
                Err(HiddenAtOpen)
            );
            assert_eq!(game.set_screen_slot(MENU, 3, None), Err(NoSuchSlot));
            assert_eq!(
                game.set_screen_slot(MENU, 0, Some([f32::NAN, 0.0])),
                Err(BadPosition)
            );
            assert_eq!(game.set_screen_slot("other:menu", 0, None), Err(NoScreen));
            assert_eq!(game.screen_slot("other:menu", 0), None);
            // Hiding a slot hidden at open is already so.
            assert_eq!(game.set_screen_slot(MENU, 2, None), Ok(()));
            game.build_screen();
            assert_eq!(geometry(game), before);

            // With the screen closed there is no screen to change.
            game.screen = Screen::Playing;
            assert_eq!(game.set_screen_slot(MENU, 0, None), Err(NoScreen));
            assert_eq!(game.screen_slot(MENU, 0), None);
        });
    });
}

/// A bound book with an accepted catalogue keeps its catalogue and its placement window
/// through a slot move and a hide (they change no window nonce), while closing and reopening
/// the screen still gives a new window that refuses the old catalogue.
#[test]
#[ignore = "daily"]
fn a_bound_books_catalogue_survives_slot_moves_and_hides() {
    use sunrise_client::recipe_book::placement_sync::{CatalogIdentity, Frame};
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("bound", Box::new(BoundBookMod))].into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            mod_api::__private::resolve_menus(["bound:menu"]).unwrap();
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "Bound").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.custom_books = true;
            remote.joined = true;
            remote.menu.set(Some((1, "bound:menu".into())));
            game.settings.gui_scale = 1;
            game.recipe_ui.test_viewport = Some([1280, 720]);
            game.title = None;
            let open = |game: &mut Game<'_>, nonce: u64| {
                game.screen = Screen::Playing;
                game.container = Some((i32::MIN, 0, 0));
                game.mod_screen = Some(OpenNativeScreen {
                    slot_moves: Default::default(),
                    layout_revision: nonce,
                    key: (i32::MIN, 0, 0),
                    window: 1,
                    menu: "bound:menu".into(),
                    block: Some("bound:block".into()),
                    mod_type: None,
                    data: Vec::new(),
                    extra: None,
                    screen: mod_api::RecipeBookLayout::registered_screen("bound:menu").unwrap(),
                });
                game.set_screen(Screen::Inventory);
            };
            game.native_net.placement.client.hello(6);
            assert!(
                game.native_net
                    .placement
                    .client
                    .ready(6, sunrise_client::recipe_book::placement_sync::schema())
            );
            open(game, 4);
            let window = game.native_net.placement.client.window().unwrap().clone();
            assert_eq!(window.nonce, 4);

            // The server's catalogue for this window, received whole and accepted.
            let client = &mut game.native_net.placement.client;
            let Some(Frame::Catalog { epoch, number, .. }) = client.open(window.clone()) else {
                panic!("no catalogue request")
            };
            let identity = CatalogIdentity {
                epoch,
                number,
                window: window.window,
                nonce: window.nonce,
                instance: 1,
                generation: 1,
                menu: window.menu.clone(),
                book: window.book.clone(),
            };
            let (_, _, layout) = mod_api::RecipeBookLayout::registered("bound:menu").unwrap();
            client.start(identity.clone(), layout, 1).unwrap();
            assert!(client.part(epoch, number, 0, vec![0]).unwrap().is_some());
            assert!(client.accept_catalog(identity.clone()));
            let kept = |game: &Game<'_>| {
                game.native_net.placement.client.catalog() == Some(&identity)
                    && game.native_net.placement.client.window() == Some(&window)
                    && game.native_book_placement_catalog_matches(&identity)
                    && game.native_book_placement_window_matches(&window)
            };
            assert!(kept(game));

            // The book declaration, not a screen part, gives the mod its slot controls.
            {
                let _call =
                    mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
                        id: "bound",
                        hook: "render_gui_post",
                        part: None,
                    });
                assert_eq!(game.screen_slot("bound:menu", 0), Some([8.0, 18.0]));
                assert_eq!(
                    game.set_screen_slot("bound:menu", 0, Some([30.0, 40.0])),
                    Ok(())
                );
                assert_eq!(game.set_screen_slot("bound:menu", 1, None), Ok(()));
            }
            game.build_screen();
            game.check_mod_book_placement_timeout();
            game.ensure_mod_book_catalog();
            assert!(kept(game));
            assert_eq!(game.native_screen_slot(0), Some((30.0, 40.0)));
            assert_eq!(game.native_screen_slot(1), None);

            // Close and reopen: a new screen, a new nonce, and the old catalogue no longer fits.
            open(game, 5);
            assert!(!game.native_book_placement_catalog_matches(&identity));
            assert_eq!(
                game.native_net.placement.client.window().map(|w| w.nonce),
                Some(5)
            );
            assert_eq!(game.native_net.placement.client.catalog(), None);
            // And the reopened screen starts from its declared layout.
            assert_eq!(game.native_screen_slot(1), Some((80.0, 18.0)));
        });
    });
}
