use super::{Dim, DisplayContext, ItemStack, OwnerState, living_owner_at, owner_at};
use crate::game::api_values::TryIntoApi;
use crate::game::native_entity_refs::tests::{with_client_game, with_game};
use serde_json::{Value, json};
use sunrise_assets::item_model::{ClientItem, Context, ItemModels, LayerContent, NoEnv, StackView};

fn model_fixture(model: Value) -> (ItemModels, StackView) {
    let mut models = ItemModels::default();
    let item = ClientItem::from_json(&json!({"model": model}).to_string()).unwrap();
    models.definitions.insert("test:probe".into(), item);
    let mut stack = StackView::new("test:probe", 1);
    stack
        .patch
        .insert("minecraft:item_model".into(), Some(json!("test:probe")));
    (models, stack)
}

#[test]
fn living_only_item_selects_fall_back_for_dropped_and_framed_items() {
    let leaf = |id| json!({"type": "minecraft:model", "model": id});
    let (models, stack) = model_fixture(json!({
        "type": "minecraft:composite",
        "models": [
            {
                "type": "minecraft:select",
                "property": "minecraft:main_hand",
                "cases": [
                    {"when": "left", "model": leaf("test:left")},
                    {"when": "right", "model": leaf("test:right")}
                ],
                "fallback": leaf("test:no_hand")
            },
            {
                "type": "minecraft:select",
                "property": "minecraft:context_entity_type",
                "cases": [
                    {"when": "minecraft:item", "model": leaf("test:item_entity")},
                    {"when": "minecraft:item_frame", "model": leaf("test:item_frame")},
                    {"when": "minecraft:item_display", "model": leaf("test:item_display")},
                    {"when": "minecraft:player", "model": leaf("test:player")},
                    {"when": "minecraft:zombie", "model": leaf("test:zombie")}
                ],
                "fallback": leaf("test:no_entity")
            }
        ]
    }));
    for (owner, display, hand, entity) in [
        (
            owner_at([0.0; 3], 0.0, "minecraft:item"),
            DisplayContext::Ground,
            "test:no_hand",
            "test:no_entity",
        ),
        (
            owner_at([0.0; 3], 0.0, "minecraft:item_frame"),
            DisplayContext::Fixed,
            "test:no_hand",
            "test:no_entity",
        ),
        (
            owner_at([0.0; 3], 0.0, "minecraft:item_display"),
            DisplayContext::Fixed,
            "test:no_hand",
            "test:no_entity",
        ),
        (
            living_owner_at([0.0; 3], 0.0, "minecraft:player", true, None),
            DisplayContext::Gui,
            "test:left",
            "test:player",
        ),
        (
            living_owner_at([0.0; 3], 0.0, "minecraft:zombie", false, None),
            DisplayContext::ThirdPersonRightHand,
            "test:right",
            "test:zombie",
        ),
    ] {
        let ctx = Context {
            display,
            owner: Some(&owner),
            level: None,
            seed: 0,
        };
        let resolved = models.resolve(&stack, &ctx, &NoEnv);
        assert_eq!(
            resolved
                .layers
                .into_iter()
                .map(|layer| layer.content)
                .collect::<Vec<_>>(),
            [hand, entity].map(|model| LayerContent::Model {
                model: model.to_owned()
            }),
            "{}",
            owner.entity_type
        );
    }
}

#[test]
#[ignore = "daily"]
fn item_team_tint_uses_the_living_owners_scoreboard_color() {
    crate::game::in_own_process(
        "game::item_draw::owner_tests::item_team_tint_uses_the_living_owners_scoreboard_color",
        || {
            with_game(|game| {
                let (models, stack) = model_fixture(json!({
                    "type": "minecraft:model",
                    "model": "test:tinted",
                    "tints": [{"type": "minecraft:team", "default": 0x12_3456}]
                }));
                let tint = |owner: &OwnerState| {
                    let ctx = Context {
                        display: DisplayContext::Gui,
                        owner: Some(owner),
                        level: None,
                        seed: 0,
                    };
                    models.resolve(&stack, &ctx, &NoEnv).layers[0].tints[0] as u32
                };
                let player_name = game.acting_player_name().to_owned();
                assert_eq!(tint(&game.player_owner(None)), 0xff12_3456);
                game.score.board.add_team("test");
                assert!(game.score.board.add_to_team(&player_name, "test"));
                assert_eq!(tint(&game.player_owner(None)), 0xff12_3456);
                game.score.board.team_mut("test").unwrap().color = Some("red".into());
                let player_owner = game.player_owner(None);
                assert_eq!(tint(&player_owner), 0xffff_5555);
                // Mod API exposes team_color as RGB24 (`0xRRGGBB`), while the built-in tint
                // source keeps opaque ARGB.
                assert_eq!(
                    player_owner.try_into_api().unwrap().team_color,
                    Some(0x00ff_5555)
                );

                let id = game
                    .spawn_mob_raw("minecraft:zombie", [0.5, 64.0, 0.5], true)
                    .unwrap();
                let holder = sunrise_core::uuid::uuid_text(game.persist.uuid_of(id).unwrap());
                assert!(game.score.board.add_to_team(&holder, "test"));
                let mob = game.mobs.iter().find(|mob| mob.id == id).unwrap();
                assert_eq!(tint(&game.mob_item_owner(mob, false)), 0xffff_5555);

                // A non-living OwnerState must not supply a team to the tint source.
                let mut frame = owner_at([0.0; 3], 0.0, "minecraft:item_frame");
                frame.team_color = Some(0xffff_5555_u32 as i32);
                assert_eq!(tint(&frame), 0xff12_3456);
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn item_draw_uses_the_stack_duration_even_when_its_model_is_replaced() {
    use std::sync::Arc;
    use sunrise_core::nbt::Tag;

    crate::game::in_own_process(
        "game::item_draw::owner_tests::item_draw_uses_the_stack_duration_even_when_its_model_is_replaced",
        || {
            with_client_game(|game| {
                let mut horn = ItemStack::new("minecraft:goat_horn", 1);
                horn.data_mut().set_other(
                    "minecraft:instrument",
                    Some(Tag::compound([
                        (
                            "sound_event",
                            Tag::String("minecraft:item.goat_horn.sound.0".into()),
                        ),
                        ("use_duration", Tag::Float(2.37_f32.to_bits())),
                        ("range", Tag::Float(16.0_f32.to_bits())),
                        ("description", Tag::String("Test horn".into())),
                    ])),
                );
                let mut bow = ItemStack::new("minecraft:bow", 1);
                bow.data_mut().set_other(
                    "minecraft:item_model",
                    Some(Tag::String("test:bow_model".into())),
                );
                for (stack, duration) in [
                    (ItemStack::new("minecraft:spyglass", 1), 1200),
                    (ItemStack::new("minecraft:bundle", 1), 200),
                    (horn, 47),
                    (bow, 72_000),
                    (ItemStack::new("minecraft:shield", 1), 72_000),
                    (ItemStack::new("minecraft:bread", 1), 32),
                    (ItemStack::new("minecraft:stick", 1), 0),
                ] {
                    let other_slot = (game.selected + 1) % 9;
                    game.inventory[game.selected] = Some(stack.clone());
                    game.inventory[other_slot] = Some(stack.clone());
                    game.carried = Some(stack.clone());
                    game.using = Some((game.selected, 0));
                    let active = game.player_owner(game.inventory[game.selected].as_ref());
                    assert!(active.using_this_item);
                    assert!(active.is_selected);
                    assert!(!active.is_carried);
                    assert_eq!(active.use_duration, duration, "{}", stack.item);

                    let inactive = game.player_owner(game.inventory[other_slot].as_ref());
                    assert!(!inactive.using_this_item);
                    assert!(!inactive.is_selected);
                    assert!(!inactive.is_carried);

                    let carried = game.player_owner(game.carried.as_ref());
                    assert!(!carried.using_this_item);
                    assert!(!carried.is_selected);
                    assert!(carried.is_carried);
                }

                // Active vs equal-but-inactive stacks resolve differently in GUI models.
                let leaf = |id| json!({"type": "minecraft:model", "model": id});
                let (models, view) = model_fixture(json!({
                    "type": "minecraft:composite",
                    "models": [
                        {
                            "type": "minecraft:condition",
                            "property": "minecraft:using_item",
                            "on_true": leaf("test:using"),
                            "on_false": leaf("test:idle")
                        },
                        {
                            "type": "minecraft:condition",
                            "property": "minecraft:selected",
                            "on_true": leaf("test:selected"),
                            "on_false": leaf("test:unselected")
                        },
                        {
                            "type": "minecraft:condition",
                            "property": "minecraft:carried",
                            "on_true": leaf("test:carried"),
                            "on_false": leaf("test:not_carried")
                        }
                    ]
                }));
                let resolve_layers = |owner: &OwnerState| {
                    let ctx = Context {
                        display: DisplayContext::Gui,
                        owner: Some(owner),
                        level: None,
                        seed: 0,
                    };
                    models
                        .resolve(&view, &ctx, &NoEnv)
                        .layers
                        .into_iter()
                        .map(|layer| layer.content)
                        .collect::<Vec<_>>()
                };
                let sample = ItemStack::new("minecraft:bow", 1);
                let other_slot = (game.selected + 1) % 9;
                game.inventory[game.selected] = Some(sample.clone());
                game.inventory[other_slot] = Some(sample.clone());
                game.carried = Some(sample);
                game.using = Some((game.selected, 4));
                assert_eq!(
                    resolve_layers(&game.player_owner(game.inventory[game.selected].as_ref())),
                    ["test:using", "test:selected", "test:not_carried"].map(|m| {
                        LayerContent::Model {
                            model: m.to_owned(),
                        }
                    })
                );
                assert_eq!(
                    resolve_layers(&game.player_owner(game.inventory[other_slot].as_ref())),
                    ["test:idle", "test:unselected", "test:not_carried"].map(|m| {
                        LayerContent::Model {
                            model: m.to_owned(),
                        }
                    })
                );
                assert_eq!(
                    resolve_layers(&game.player_owner(game.carried.as_ref())),
                    ["test:idle", "test:unselected", "test:carried"].map(|m| LayerContent::Model {
                        model: m.to_owned(),
                    })
                );

                // Third-person held_item_geometry supplies the stack's real use duration rather
                // than a hardcoded 72_000 for bread (32) and spyglass (1200).
                let probe = ClientItem::from_json(
                    &json!({
                        "model": {
                            "type": "minecraft:range_dispatch",
                            "property": "minecraft:use_duration",
                            "remaining": true,
                            "entries": [
                                {"threshold": 0.0, "model": leaf("minecraft:block/stone")},
                                {"threshold": 100.0, "model": leaf("minecraft:block/oak_stairs")},
                                {"threshold": 10000.0, "model": leaf("minecraft:block/oak_slab")}
                            ],
                            "fallback": leaf("minecraft:block/stone")
                        }
                    })
                    .to_string(),
                )
                .unwrap();
                Arc::get_mut(&mut game.pack.item_renderer)
                    .expect("fresh test item renderer")
                    .models
                    .definitions
                    .insert("test:third_person_use_probe".into(), probe);
                let mut mob = sunrise_client::mobs::Mob::new(
                    77,
                    game.res
                        .mob_types()
                        .get("minecraft:zombie")
                        .unwrap()
                        .clone(),
                    sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
                    0.0,
                );
                mob.extra.using_item = 5;
                mob.extra.using_off_hand = false;
                let draw_held = |game: &crate::game::Game<'_>,
                                 mob: &mut sunrise_client::mobs::Mob,
                                 item: &str| {
                    let mut stack = ItemStack::new(item, 1);
                    stack.data_mut().set_other(
                        "minecraft:item_model",
                        Some(Tag::String("test:third_person_use_probe".into())),
                    );
                    mob.extra
                        .set_slot_stack(sunrise_items::equipment::MAINHAND, Some(stack));
                    let mut verts = Vec::new();
                    game.held_item_geometry(
                        mob,
                        glam::DVec3::new(0.5, 65.0, 3.5),
                        0.0,
                        0xff,
                        true,
                        None,
                        &mut verts,
                    );
                    verts.len()
                };
                let bread_verts = draw_held(game, &mut mob, "minecraft:bread");
                let spyglass_verts = draw_held(game, &mut mob, "minecraft:spyglass");
                let bow_verts = draw_held(game, &mut mob, "minecraft:bow");
                assert!(bread_verts > 0);
                assert!(spyglass_verts > 0);
                assert!(bow_verts > 0);
                assert_ne!(
                    bread_verts, spyglass_verts,
                    "bread (remaining 28) and spyglass (remaining 1196) must pick different duration thresholds"
                );
                assert_ne!(
                    spyglass_verts, bow_verts,
                    "spyglass (remaining 1196) must not hit the 72000 threshold"
                );
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn item_display_passes_non_living_owner_with_position_and_yaw() {
    use crate::game::text_displays::{RemoteTextDisplay, Shows};
    use std::sync::Arc;
    use sunrise_core::nbt::Tag;

    crate::game::in_own_process(
        "game::item_draw::owner_tests::item_display_passes_non_living_owner_with_position_and_yaw",
        || {
            with_client_game(|game| {
                let leaf = |id| json!({"type": "minecraft:model", "model": id});
                let living_probe = ClientItem::from_json(
                    &json!({
                        "model": {
                            "type": "minecraft:select",
                            "property": "minecraft:context_entity_type",
                            "cases": [
                                {"when": "minecraft:item_display", "model": leaf("minecraft:block/oak_stairs")}
                            ],
                            "fallback": leaf("minecraft:block/stone")
                        }
                    })
                    .to_string(),
                )
                .unwrap();
                let compass_probe = ClientItem::from_json(
                    &json!({
                        "model": {
                            "type": "minecraft:range_dispatch",
                            "property": "minecraft:compass",
                            "target": "spawn",
                            "wobble": false,
                            "entries": [
                                {"threshold": 0.0, "model": leaf("minecraft:block/stone")},
                                {"threshold": 0.5, "model": leaf("minecraft:block/oak_stairs")}
                            ],
                            "fallback": leaf("minecraft:block/stone")
                        }
                    })
                    .to_string(),
                )
                .unwrap();
                let defs = &mut Arc::get_mut(&mut game.pack.item_renderer)
                    .expect("fresh test item renderer")
                    .models
                    .definitions;
                defs.insert("test:display_living_probe".into(), living_probe);
                defs.insert("test:display_compass_probe".into(), compass_probe);

                game.dim = Dim::Overworld;
                game.spawn = Some(sunrise_core::geometry::Vec3::new(0.0, 64.0, 0.0));
                game.view_frustum = None;

                let draw_display =
                    |game: &mut crate::game::Game<'_>, id: i32, yaw: f32, model_id: &str| {
                        game.text_displays.clear();
                        let mut stack = ItemStack::new("minecraft:compass", 1);
                        stack
                            .data_mut()
                            .set_other("minecraft:item_model", Some(Tag::String(model_id.into())));
                        let mut display =
                            RemoteTextDisplay::new(Shows::Item, [10.0, 64.0, 0.0], yaw);
                        display.show(&[], Some(Some(stack)));
                        display.tick();
                        game.text_displays.insert(id, display);
                        let mut out = Vec::new();
                        game.block_item_display_geometry(
                            glam::DVec3::new(10.0, 64.0, 3.0),
                            0.0,
                            &mut out,
                        );
                        out.len()
                    };

                // Living-only select falls back to `stone` (24 vertices), never matching item_display.
                let stone_count = draw_display(game, 10, 0.0, "test:display_living_probe");
                assert_eq!(stone_count, 24);

                // Compass target depends on the display's position and visual Y rotation: yaw 0 vs
                // yaw 180 flip the needle across the 0.5 threshold.
                let yaw_0 = draw_display(game, 11, 0.0, "test:display_compass_probe");
                let yaw_180 = draw_display(game, 12, 180.0, "test:display_compass_probe");
                assert!(yaw_0 > 0 && yaw_180 > 0);
                assert_ne!(
                    yaw_0, yaw_180,
                    "item display compass must use the display's position and yaw"
                );
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn item_moon_phase_follows_the_dimension_timeline() {
    let plain = |game: &mut crate::game::Game<'_>, item: &str, owner: &OwnerState| {
        game.resolve_item(
            &ItemStack::new(item, 1),
            DisplayContext::Ground,
            Some(owner),
            0,
        )
    };
    use std::sync::Arc;
    use sunrise_client::{chunk::Chunk, lan::data::json_to_nbt};
    use sunrise_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C},
    };

    crate::game::in_own_process(
        "game::item_draw::owner_tests::item_moon_phase_follows_the_dimension_timeline",
        || {
            with_client_game(|game| {
                game.world.day_time = 24_000 * 3;
                for (dim, phase) in [(Dim::Nether, 0), (Dim::End, 0), (Dim::Overworld, 3)] {
                    game.dim = dim;
                    assert_eq!(game.item_level().moon_phase, phase, "{}", dim.id());
                }

                // A server can sync a custom timeline and custom biomes with visual/moon_phase
                // attributes; build_level_environments must read both from the synced registries
                // and resolve per-owner biome moon phases within the same frame.
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "MoonTimeline").unwrap();
                let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
                registries.registries.insert(
                    "minecraft:world_clock".into(),
                    vec![("minecraft:overworld".into(), None)],
                );
                let custom_timeline = json!({
                    "clock": "minecraft:overworld",
                    "period_ticks": 192_000,
                    "tracks": {
                        "visual/moon_phase": {
                            "keyframes": [
                                {"ticks": 0, "value": "full_moon"},
                                {"ticks": 24_000, "value": "waning_gibbous"},
                                {"ticks": 48_000, "value": "third_quarter"},
                                {"ticks": 72_000, "value": "waning_crescent"}
                            ]
                        }
                    }
                });
                registries.registries.insert(
                    "minecraft:timeline".into(),
                    vec![(
                        "test:server_moon".into(),
                        Some(json_to_nbt(&custom_timeline)),
                    )],
                );
                let biome_json = |phase: Option<&str>| {
                    let mut obj = json!({
                        "has_precipitation": false,
                        "temperature": 0.5,
                        "downfall": 0.0,
                        "effects": {
                            "water_color": 4_159_204
                        }
                    });
                    if let Some(phase) = phase {
                        obj["attributes"] = json!({"visual/moon_phase": phase});
                    }
                    json_to_nbt(&obj)
                };
                registries.registries.insert(
                    "minecraft:worldgen/biome".into(),
                    vec![
                        ("minecraft:plains".into(), Some(biome_json(None))),
                        (
                            "test:moon_two".into(),
                            Some(biome_json(Some("third_quarter"))),
                        ),
                        (
                            "test:moon_six".into(),
                            Some(biome_json(Some("waxing_gibbous"))),
                        ),
                    ],
                );
                let dim_type = json!({
                    "timelines": ["test:server_moon"]
                });
                let no_timeline = json!({
                    "timelines": []
                });
                registries.registries.insert(
                    "minecraft:dimension_type".into(),
                    vec![
                        ("minecraft:the_end".into(), Some(json_to_nbt(&dim_type))),
                        (
                            "minecraft:overworld".into(),
                            Some(json_to_nbt(&no_timeline)),
                        ),
                    ],
                );
                game.set_registries(Arc::new(registries));
                let spawn_in = |dimension_type: i32, dimension: &str| play::SpawnInfo {
                    dimension_type,
                    dimension: dimension.into(),
                    seed: 0,
                    game_type: 1,
                    previous_game_type: None,
                    debug: false,
                    flat: true,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                };
                let respawn =
                    |game: &mut crate::game::Game<'_>, dimension_type: i32, dimension: &str| {
                        game.handle_packet(C::Respawn(play::Respawn {
                            spawn: spawn_in(dimension_type, dimension),
                            keep: 0,
                        }))
                        .unwrap();
                    };
                let lay_biomes = |game: &mut crate::game::Game<'_>| {
                    game.world.chunks.insert(
                        (0, 0),
                        Arc::new(Chunk::with_layout(0, 0, game.world.layout())),
                    );
                    assert!(game.set_biome_at([0, 64, 0], "test:moon_two"));
                    assert!(game.set_biome_at([8, 64, 0], "test:moon_six"));
                };
                respawn(game, 0, "minecraft:the_end");
                game.handle_packet(C::SetTime(play::SetTime {
                    game_time: 100,
                    clocks: vec![play::ClockState {
                        clock: 0,
                        total_ticks: 24_000 * 3,
                        partial_tick: 0.0,
                        rate: 1.0,
                    }],
                }))
                .unwrap();
                game.world.day_time = 24_000 * 6;

                // Dimension-level moon phase comes from the synced server-only timeline (tick 72000 -> 3).
                assert_eq!(game.item_level().moon_phase, 3);
                assert!(game.dim_environment().moon_phase_is_positional());

                // Per-owner lookups in the same frame, without advancing `frames`.
                lay_biomes(game);
                let leaf = |id| json!({"type": "minecraft:model", "model": id});
                let moon_probe = ClientItem::from_json(
                    &json!({
                        "model": {
                            "type": "minecraft:range_dispatch",
                            "property": "minecraft:time",
                            "source": "moon_phase",
                            "wobble": false,
                            "entries": [
                                {"threshold": 0.0, "model": leaf("minecraft:item/iron_ingot")},
                                {"threshold": 0.25, "model": leaf("minecraft:item/diamond")},
                                {"threshold": 0.5, "model": leaf("minecraft:item/emerald")},
                                {"threshold": 0.75, "model": leaf("minecraft:item/gold_ingot")}
                            ],
                            "fallback": leaf("minecraft:item/iron_ingot")
                        }
                    })
                    .to_string(),
                )
                .unwrap();
                Arc::get_mut(&mut game.pack.item_renderer)
                    .expect("fresh test item renderer")
                    .models
                    .definitions
                    .insert("test:moon_probe".into(), moon_probe);
                let mut stack = ItemStack::new("minecraft:clock", 1);
                stack.data_mut().set_other(
                    "minecraft:item_model",
                    Some(sunrise_core::nbt::Tag::String("test:moon_probe".into())),
                );
                let owner_a = owner_at([0.5, 64.0, 0.5], 0.0, "minecraft:item");
                let owner_b = owner_at([8.5, 64.0, 0.5], 0.0, "minecraft:item");
                let resolved = |game: &mut crate::game::Game<'_>, owner: &OwnerState| {
                    game.resolve_item(&stack, DisplayContext::Ground, Some(owner), 0)
                };
                // The chosen leaf is the layer's baked geometry: compare it with plain items
                // that use the same models.
                let quads_of = |state: &sunrise_client::item_render::ItemRenderState| match &state
                    .layers[0]
                    .geometry
                {
                    sunrise_client::item_render::LayerGeometry::Quads(baked) => baked.quads.clone(),
                    other => panic!("expected a plain model, got {other:?}"),
                };
                let diamond = quads_of(&plain(game, "minecraft:diamond", &owner_a));
                let gold = quads_of(&plain(game, "minecraft:gold_ingot", &owner_b));
                assert_ne!(diamond, gold);

                // A timeline track replaces what the layers before it gave:
                // `Timeline.Builder.addTrack` is `AttributeModifier.override()`, and
                // `addDefaultLayers` adds the dimension type's timelines after the biome
                // layer. Both owners take the timeline's phase, whatever their biomes say.
                assert_eq!(game.item_moon_phase(Some([0.5, 64.0, 0.5])), 3);
                assert_eq!(game.item_moon_phase(Some([8.5, 64.0, 0.5])), 3);
                assert_eq!(
                    quads_of(&resolved(game, &owner_a)),
                    diamond,
                    "owner a takes the timeline's phase"
                );
                assert_eq!(
                    quads_of(&resolved(game, &owner_b)),
                    diamond,
                    "owner b takes the timeline's phase"
                );

                // A dimension type without the track takes each owner's biome phase
                // (`addBiomeLayerForAttribute`: the biome at the owner's position).
                respawn(game, 1, "minecraft:overworld");
                lay_biomes(game);
                assert_eq!(game.item_level().moon_phase, 0, "no track: the default");
                assert!(game.dim_environment().moon_phase_is_positional());
                assert_eq!(game.item_moon_phase(Some([0.5, 64.0, 0.5])), 2);
                assert_eq!(game.item_moon_phase(Some([8.5, 64.0, 0.5])), 7);
                assert_eq!(quads_of(&resolved(game, &owner_a)), diamond);
                assert_eq!(quads_of(&resolved(game, &owner_b)), gold);
            })
        },
    );
}
