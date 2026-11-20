//! The render hook pair around a living entity (#3681): `render_living_pre` and
//! `render_living_post`, wrapped for a player by `render_player_pre` and `render_player_post`. A
//! canceled pre hook skips the entity's model, its layers, its post hooks and the frame's later
//! passes; a culled entity is asked neither; the hooks run in the draw queue's order.
use super::Game;
use mod_api::{
    BlockEntityDrawContext, Hooks, RenderLivingPostEvent, RenderLivingPreEvent,
    RenderPlayerPostEvent, RenderPlayerPreEvent,
};
use std::sync::{Arc, Mutex};
use sunrise_client::{mesher::Vertex, mobs::Mob};

/// What the mod heard, in order, and which entities it cancels.
#[derive(Default)]
struct Heard {
    events: Vec<String>,
    /// Entity ids whose `render_living_pre` it cancels.
    cancel_living: Vec<u32>,
    /// Whether it cancels `render_player_pre` of this window's own player (`true`) or another.
    cancel_player: Vec<bool>,
}

/// A mod listening to the four hooks: it records each call and, in each pre hook, draws one stone
/// quad 100 blocks above the entity, so a test sees what a canceled entity still draws.
struct Pair(Arc<Mutex<Heard>>);

fn marker(draw: &mut dyn BlockEntityDrawContext, height: f32) {
    let corners = [
        [0.0, height + 1.0, 0.0],
        [0.0, height, 0.0],
        [1.0, height, 0.0],
        [1.0, height + 1.0, 0.0],
    ];
    draw.quad("block/stone", corners, [0.0, 0.0, 16.0, 16.0], 0xFF_FFFF);
}

#[mod_api::__subscriptions]
impl Hooks for Pair {
    fn render_player_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderPlayerPreEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.0.lock().unwrap();
        heard.events.push(format!("player_pre {}", event.who.local));
        event.cancelled = heard.cancel_player.contains(&event.who.local);
        mod_api::Flow::Continue
    }
    fn render_living_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderLivingPreEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.0.lock().unwrap();
        heard.events.push(format!("living_pre {}", event.entity.id));
        marker(event.draw, 100.0);
        event.cancelled = heard.cancel_living.contains(&event.entity.id);
        mod_api::Flow::Continue
    }
    fn render_living_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderLivingPostEvent<'_>,
    ) {
        let mut heard = self.0.lock().unwrap();
        heard
            .events
            .push(format!("living_post {}", event.entity.id));
    }
    fn render_player_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderPlayerPostEvent<'_>,
    ) {
        let mut heard = self.0.lock().unwrap();
        heard
            .events
            .push(format!("player_post {}", event.who.local));
    }
}

/// The camera, south of the entities and looking at them.
const CAMERA: glam::DVec3 = glam::DVec3::new(0.5, 64.0, -4.5);

/// A mob of `kind` with id `id`, standing `x` blocks east of the camera's line.
fn mob(game: &Game<'_>, kind: &str, id: u32, x: f64) -> Mob {
    let kind = if kind == "minecraft:player" {
        game.res.player_type().clone().unwrap()
    } else {
        game.res.mob_types().get(kind).unwrap().clone()
    };
    let at = sunrise_core::geometry::Vec3::new(0.5 + x, 64.0, 0.5);
    Mob::new(id, kind, at, 0.0)
}

/// How many of `out`'s vertices are 100 or more blocks up: the pre hooks' markers.
fn markers(out: &[Vertex]) -> usize {
    out.iter()
        .filter(|v| v.pos[1] - (64.0 - CAMERA.y) as f32 >= 99.0)
        .count()
}

/// One frame of the level's mobs, begun as the entity pass begins it.
fn mob_frame(game: &mut Game<'_>) -> Vec<Vertex> {
    game.prepare_entity_culling();
    let mut out = Vec::new();
    game.mob_geometry(CAMERA, 1.0, &mut out);
    out
}

/// Runs `check` on a client game with prepared resources and the [`Pair`] mod.
fn with_pair(heard: &Arc<Mutex<Heard>>, check: impl FnOnce(&mut Game<'_>)) {
    let mods = vec![mod_api::ModEntry::new(
        "pair",
        Box::new(Pair(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::native_entity_refs::tests::with_client_game(|game| {
            game.mobs.clear();
            check(game);
        });
    });
}

fn events(heard: &Arc<Mutex<Heard>>) -> Vec<String> {
    std::mem::take(&mut heard.lock().unwrap().events)
}

#[test]
#[ignore = "daily"]
fn two_entities_are_asked_in_the_draw_queues_order() {
    let heard = Arc::default();
    with_pair(&heard, |game| {
        game.mobs.push(mob(game, "minecraft:zombie", 91, 0.0));
        game.mobs.push(mob(game, "minecraft:cow", 92, 2.0));
        let out = mob_frame(game);
        assert_eq!(
            events(&heard),
            [
                "living_pre 91",
                "living_post 91",
                "living_pre 92",
                "living_post 92"
            ]
        );
        assert_eq!(markers(&out), 8, "each pre hook's marker is kept");
    });
}

#[test]
#[ignore = "daily"]
fn a_canceled_living_pre_skips_the_model_and_the_post_hook() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        game.mobs.push(mob(game, "minecraft:zombie", 91, 0.0));
        let drawn = mob_frame(game);
        assert!(drawn.len() > 4, "its model, beside the marker");
        events(&heard);
        heard.lock().unwrap().cancel_living.push(91);
        let canceled = mob_frame(game);
        assert_eq!(events(&heard), ["living_pre 91"], "no post hook");
        assert_eq!(
            canceled.len(),
            4,
            "the marker alone, no model: {}",
            canceled.len()
        );
        assert_eq!(markers(&canceled), 4);
        // The frame's later passes (name tag, shadow, flames, leash) leave it out.
        assert!(game.render_canceled(91));
        assert_eq!(game.drawn_mobs().count(), 0);
    });
}

#[test]
#[ignore = "daily"]
fn a_canceled_entity_draws_no_name_tag() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        let mut named = mob(game, "minecraft:zombie", 91, 0.0);
        named.custom_name = Some("Bob".into());
        named.name_visible = true;
        game.mobs.push(named);
        let tag = |game: &mut Game<'_>| {
            mob_frame(game);
            let (mut out, mut see_through) = (Vec::new(), Vec::new());
            game.name_tag_geometry(CAMERA, 1.0, &mut out, &mut see_through);
            out.len() + see_through.len()
        };
        assert!(tag(game) > 0, "a named zombie's tag");
        heard.lock().unwrap().cancel_living.push(91);
        assert_eq!(tag(game), 0, "none once its pre hook cancels");
    });
}

#[test]
#[ignore = "daily"]
fn a_culled_entity_is_asked_neither_hook() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        // Far past any render distance.
        game.mobs.push(mob(game, "minecraft:zombie", 91, 100_000.0));
        let out = mob_frame(game);
        assert_eq!(events(&heard), Vec::<String>::new());
        assert_eq!(markers(&out), 0);
    });
}

/// This window's player in third person, drawn by the game's own path for it, with its own entity
/// id 4321; spectating when `spectator`.
fn own_player_frame(game: &mut Game<'_>, spectator: bool) -> Vec<Vertex> {
    let at = sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5);
    let mut player = sunrise_client::motion::PlayerMotion::new(game.motion, at).unwrap();
    player.entity_id = 4321;
    game.player = Some(player);
    if let Some(remote) = game.remote.as_mut() {
        remote.entity_id = 4321;
    }
    game.camera_type = sunrise_client::camera::CameraType::ThirdPersonBack;
    game.tick_player_model();
    game.player_model.as_mut().unwrap().extra.spectator = spectator;
    game.prepare_entity_culling();
    let mut out = Vec::new();
    game.player_model_geometry(CAMERA, 1.0, &mut out);
    out
}

#[test]
#[ignore = "daily"]
fn the_player_pair_wraps_the_living_pair() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        own_player_frame(game, false);
        assert_eq!(
            events(&heard),
            [
                "player_pre true",
                "living_pre 4321",
                "living_post 4321",
                "player_post true"
            ]
        );
        heard.lock().unwrap().cancel_living.push(4321);
        own_player_frame(game, false);
        assert_eq!(
            events(&heard),
            ["player_pre true", "living_pre 4321", "player_post true"],
            "a canceled living pre still asks the player post"
        );
        assert!(game.own_render_canceled());
        heard.lock().unwrap().cancel_player.push(true);
        own_player_frame(game, false);
        assert_eq!(
            events(&heard),
            ["player_pre true"],
            "a canceled player pre skips the rest"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_spectator_draws_no_layers_but_gets_its_post_hooks() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        // This window's player, spectating, in third person.
        own_player_frame(game, true);
        assert_eq!(
            events(&heard),
            [
                "player_pre true",
                "living_pre 4321",
                "living_post 4321",
                "player_post true"
            ]
        );
        // A spectating mannequin in the level's draw.
        let mut mannequin = mob(game, "minecraft:mannequin", 94, 0.0);
        mannequin.extra.spectator = true;
        game.mobs.push(mannequin);
        mob_frame(game);
        assert_eq!(events(&heard), ["living_pre 94", "living_post 94"]);
    });
}

#[test]
#[ignore = "daily"]
fn stock_hides_are_draw_local_and_refused_callbacks_cannot_erase_them() {
    use mod_api::__private::{ModPhase, PhaseOwner};
    use mod_api::{
        ArmorLayerPass, ArmorLayerSlot, ClientHost, EntityRenderLayer, EntityRenderState, Flow,
        HideLayerError, LayerTarget, PlayerModel, PosedParts, RegisterEntityLayersEvent,
        RegistrationContext, StockEntityLayer as Layer,
    };
    use sunrise_client::hud::{GuiScale, HudBatch};
    use sunrise_client::mobs::render::ARMOR_GLINT;

    const CHEST_BASE: Layer = Layer::Armor(ArmorLayerSlot::Chest, ArmorLayerPass::Base);
    const CHEST_GLINT: Layer = Layer::Armor(ArmorLayerSlot::Chest, ArmorLayerPass::Glint);
    const CHEST_TRIM: Layer = Layer::Armor(ArmorLayerSlot::Chest, ArmorLayerPass::Trim);
    type Results = Arc<Mutex<Vec<(u32, Result<(), HideLayerError>)>>>;
    struct Hide(Results);
    #[mod_api::__subscriptions]
    impl Hooks for Hide {
        fn render_player_pre(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderPlayerPreEvent<'_>,
        ) -> Flow {
            if event.who.local {
                event.hidden_layers.hide(Layer::Cape).unwrap();
                event.hidden_layers.hide(Layer::Cape).unwrap();
            }
            Flow::Continue
        }
        fn render_living_pre(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderLivingPreEvent<'_>,
        ) -> Flow {
            let id = event.entity.id;
            if matches!(id, 92 | 93) {
                let result = event.hidden_layers.hide(Layer::HeadItem);
                self.0.lock().unwrap().push((id, result));
            }
            Flow::Continue
        }
    }
    type Failures = Arc<Mutex<Vec<(&'static str, Result<(), HideLayerError>)>>>;
    struct Fail {
        panic: bool,
        seen: Failures,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Fail {
        fn render_living_pre(
            &self,
            host: &dyn ClientHost,
            event: &mut RenderLivingPreEvent<'_>,
        ) -> Flow {
            if event.entity.id != 4321 {
                return Flow::Continue;
            }
            event.cancelled = true;
            if self.panic {
                let result = event.hidden_layers.hide(CHEST_TRIM);
                self.seen.lock().unwrap().push(("panic", result));
                panic!("a failing living pre listener");
            }
            let result = event.hidden_layers.hide(CHEST_BASE);
            self.seen.lock().unwrap().push(("refusal", result));
            let _ = host.level("minecraft:overworld");
            self.seen.lock().unwrap().push(("returned", Ok(())));
            Flow::Continue
        }
    }
    struct Markers;
    impl EntityRenderLayer for Markers {
        fn submit(
            &self,
            entity: &EntityRenderState,
            _: Option<&PosedParts>,
            draw: &mut dyn BlockEntityDrawContext,
        ) -> Result<(), String> {
            if entity.id == 4321 {
                marker(draw, 200.0);
            }
            Ok(())
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for Markers {
        fn register_entity_layers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterEntityLayersEvent<'_>,
        ) {
            for model in [PlayerModel::Wide, PlayerModel::Slim] {
                event
                    .layers
                    .add(LayerTarget::Player(model), Arc::new(Self))
                    .unwrap();
            }
        }
        fn render_living_pre(
            &self,
            _: &dyn ClientHost,
            event: &mut RenderLivingPreEvent<'_>,
        ) -> Flow {
            if event.entity.id == 4321 {
                event.hidden_layers.hide(CHEST_GLINT).unwrap();
            }
            Flow::Continue
        }
        fn render_living_post(&self, _: &dyn ClientHost, event: &mut RenderLivingPostEvent<'_>) {
            if event.entity.id == 4321 {
                marker(event.draw, 300.0);
            }
        }
        fn render_player_post(&self, _: &dyn ClientHost, event: &mut RenderPlayerPostEvent<'_>) {
            if event.who.local {
                marker(event.draw, 400.0);
            }
        }
    }
    super::native_entity_refs::tests::with_client_game(|game| {
        let mut chest = sunrise_client::items::ItemStack::new("minecraft:diamond_chestplate", 1);
        chest.enchant("minecraft:protection", 1);
        chest.data_mut().trim = Some(("minecraft:gold".into(), "minecraft:sentry".into()));
        game.armor[1] = Some(chest.clone());
        game.prepare_trims();
        let trim_sprite = game
            .armor_stand_assets
            .trim_texture("minecraft:diamond", "humanoid", &chest)
            .expect("the equipped armor has a named trim")
            .sprite();
        let trim = game.gui.atlas.sprites[&trim_sprite];
        let base = game.gui.atlas.sprites["entity/equipment/humanoid/diamond"];
        own_player_frame(game, false);
        // Mark the capes with independent atlas regions, disjoint from the skin and armor.
        let local = game.gui.atlas.sprites["block/gold_block"];
        let remote = game.gui.atlas.sprites["block/diamond_block"];
        game.gui
            .atlas
            .sprites
            .insert("test:local_cape".into(), local);
        game.gui
            .atlas
            .sprites
            .insert("test:remote_cape".into(), remote);
        game.player_model.as_mut().unwrap().cape = Some("test:local_cape".into());
        let mut other = mob(game, "minecraft:player", 91, 2.0);
        other.cape = Some("test:remote_cape".into());
        let mobs = [
            other,
            mob(game, "minecraft:creeper", 92, 4.0),
            mob(game, "minecraft:creeper", 93, -4.0),
        ];
        game.mobs.clear();
        game.mobs.extend(mobs);
        let unadorned_mobs = mob_frame(game);
        game.mobs[1].extra.set_slot_id(
            sunrise_items::equipment::HEAD,
            Some("minecraft:carved_pumpkin".into()),
        );
        let inside = |uv: [f32; 2], rect: sunrise_client::hud::Rect| {
            uv[0] > rect.u0 && uv[0] < rect.u1 && uv[1] > rect.v0 && uv[1] < rect.v1
        };
        let frame = |game: &mut Game<'_>| {
            let mobs = mob_frame(game);
            let mut player = Vec::new();
            game.player_model_geometry(CAMERA, 1.0, &mut player);
            let mut hud = HudBatch::new(
                [320, 240],
                GuiScale {
                    requested: 1,
                    unicode: false,
                },
            );
            game.draw_player_preview(&mut hud, 26.0, 8.0, 75.0, 78.0, 30.0, 0.0625);
            (player, mobs, hud.vertices)
        };
        let (stock, equipped_mobs, preview) = frame(game);
        assert!(
            equipped_mobs.len() > unadorned_mobs.len(),
            "a creeper submits the head item"
        );
        assert!(unadorned_mobs.iter().any(|v| inside(v.uv, remote)));
        assert!(preview.iter().any(|v| inside(v.uv, local)));
        let quad_uses = |quad: &[Vertex; 4], sprite| {
            let uv = std::array::from_fn(|k| quad.iter().map(|v| v.uv[k]).sum::<f32>() / 4.0);
            inside(uv, sprite)
        };
        let quads = stock.as_chunks::<4>().0.iter();
        assert!(quads.clone().any(|q| quad_uses(q, local)));
        let first_base = quads
            .clone()
            .position(|q| quad_uses(q, base))
            .expect("stock armor base");
        let first_trim = quads
            .clone()
            .position(|q| quad_uses(q, trim))
            .expect("stock armor trim");
        assert!(first_base < first_trim);
        assert!(
            quads
                .clone()
                .filter(|q| q[0].layer == ARMOR_GLINT)
                .all(|q| quad_uses(q, trim))
        );
        assert_eq!(quads.clone().nth(first_trim).unwrap()[0].layer, ARMOR_GLINT);
        let expected: Vec<_> = quads
            .filter(|q| !quad_uses(q, local))
            .flatten()
            .map(|v| {
                if v.layer == ARMOR_GLINT {
                    Vertex {
                        layer: 0,
                        glint_uv: [0.0; 2],
                        ..*v
                    }
                } else {
                    *v
                }
            })
            .collect();

        let results = Results::default();
        let failures = Failures::default();
        let _setup = PhaseOwner::scope(ModPhase::Setup);
        mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new("hide", Box::new(Hide(results.clone()))),
                mod_api::ModEntry::new(
                    "refused",
                    Box::new(Fail {
                        panic: false,
                        seen: failures.clone(),
                    }),
                ),
                mod_api::ModEntry::new(
                    "panicked",
                    Box::new(Fail {
                        panic: true,
                        seen: failures.clone(),
                    }),
                ),
                mod_api::ModEntry::new("markers", Box::new(Markers)),
            ]
            .into(),
            || {
                game.resolve_entity_renderers();
                let (actual, actual_mobs, actual_preview) = frame(game);
                let (stock, markers) = actual.split_at(expected.len());
                assert_eq!(
                    bytemuck::cast_slice::<_, u8>(stock),
                    bytemuck::cast_slice::<_, u8>(&expected)
                );
                let heights: Vec<_> = markers
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|q| q.iter().map(|v| v.pos[1]).reduce(f32::min).unwrap())
                    .collect();
                assert_eq!(markers.len(), 12);
                assert_eq!(
                    heights,
                    [200.0, 300.0, 400.0],
                    "registered layer, Living Post, Player Post"
                );
                assert_eq!(
                    bytemuck::cast_slice::<_, u8>(&actual_mobs),
                    bytemuck::cast_slice::<_, u8>(&unadorned_mobs)
                );
                assert_eq!(
                    bytemuck::cast_slice::<_, u8>(&actual_preview),
                    bytemuck::cast_slice::<_, u8>(&preview)
                );
                assert_eq!(game.armor[1].as_ref(), Some(&chest));
                assert_eq!(
                    game.mobs[1].extra.head_id(),
                    Some("minecraft:carved_pumpkin")
                );
            },
        );
        assert_eq!(*results.lock().unwrap(), [(92, Ok(())), (93, Ok(()))]);
        assert_eq!(
            *failures.lock().unwrap(),
            [("refusal", Ok(())), ("panic", Ok(()))]
        );
        game.resolve_entity_renderers();
    });
}
