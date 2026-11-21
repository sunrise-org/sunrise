//! What special model renderers cost the frame (matrix row 13 of
//! `docs/design/mod-api/resource-renderers.md`, and the measurement in
//! `docs/performance/special-item-renderers.md`). Each test runs alone in its own process, the
//! measurement by hand: it registers the process's renderers.
use crate::game::Game;
use mod_api::{ClientHost, SpecialModelPart, SpecialModelRenderer};
use std::sync::Arc;
use sunrise_assets::item_model::{ClientItem, DisplayContext, register_special_model_types};
use sunrise_client::{item_render::ItemRenderState, items::ItemStack, mesher::Vertex};

/// Full sky and block light, as the frame packs it.
const LIGHT: u32 = 0x00F0_00F0;

/// A built-in scene: a plain model, a tinted one and vanilla's own special models.
const SCENE: [&str; 6] = [
    "minecraft:stick",
    "minecraft:leather_chestplate",
    "minecraft:shield",
    "minecraft:chest",
    "minecraft:red_bed",
    "minecraft:skeleton_skull",
];

/// `stack` resolved and submitted as a frame draws an item in the world, into `out`.
fn draw(game: &Game<'_>, stack: &ItemStack, out: &mut Vec<Vertex>) -> ItemRenderState {
    let state = game.resolve_item(stack, DisplayContext::Gui, None, 0);
    game.submit_item_unlit(&state, glam::Mat4::IDENTITY, LIGHT, out);
    state
}

/// The game's GUI atlas with only the missing-texture sprite, which every quad then draws
/// with: the test game has no window to build the atlas in.
fn give_every_quad_a_sprite(game: &mut Game<'_>) {
    let whole = sunrise_client::hud::Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    };
    game.gui
        .atlas
        .sprites
        .insert("misc/missingno".into(), whole);
}

/// One frame of the scene: the frame's start, then every item, with the allocations each
/// made and the vertices drawn.
fn frame(game: &Game<'_>, stacks: &[ItemStack], out: &mut Vec<Vertex>) -> (usize, usize) {
    out.clear();
    let ((), started) = sunrise_threads::allocation_count::measure(|| game.begin_frame_draws());
    let ((), drawn) = sunrise_threads::allocation_count::measure(|| {
        for stack in stacks {
            draw(game, stack, out);
        }
    });
    (started, drawn)
}

/// Never asked: no drawn item's model is of its type.
struct Unmatched;
impl SpecialModelRenderer for Unmatched {
    fn extract(
        &self,
        _: &dyn ClientHost,
        _: &mod_api::ItemStack,
        _: &sunrise_core::nbt::Tag,
        _: &mod_api::SpecialModelContext,
    ) -> Result<Vec<SpecialModelPart>, String> {
        panic!("a renderer no drawn model names was asked")
    }
}

/// Row 13: with no special renderer registered, a built-in scene drawn for 10 frames draws the
/// same vertices every frame, keeps no answer, starts each frame with no allocation, and
/// allocates as many times each frame as the frame before. Vanilla's resolver allocates for
/// its own layers, so the scene can't be 0 as a whole; nothing the extension adds grows from
/// frame to frame. A renderer registered for a type no drawn model names changes none of it,
/// down to the allocation count.
#[test]
#[ignore = "daily"]
fn built_in_scenes_cost_nothing_more_without_a_listener() {
    crate::game::in_own_process(
        "game::native_special_renderers::cost_tests::built_in_scenes_cost_nothing_more_without_a_listener",
        built_in_scenes_cost_nothing_more_without_a_listener_isolated,
    );
}

fn built_in_scenes_cost_nothing_more_without_a_listener_isolated() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        let stacks: Vec<ItemStack> = SCENE.iter().map(|id| ItemStack::new(id, 1)).collect();
        give_every_quad_a_sprite(game);
        let mut out = Vec::new();
        // Ten frames, the first warming the game's own caches: what each steady frame allocated
        // and the vertices of the last.
        let mut scene = |game: &Game<'_>| {
            frame(game, &stacks, &mut out);
            let first = bytemuck::cast_slice::<Vertex, u8>(&out).to_vec();
            assert!(!first.is_empty(), "the scene drew nothing");
            let mut counts = Vec::new();
            for _ in 1..10 {
                let (started, drawn) = frame(game, &stacks, &mut out);
                assert_eq!(started, 0, "the frame's start allocated");
                assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&out), first);
                assert_eq!(super::kept_answers(), 0);
                counts.push(drawn);
            }
            assert!(
                counts.windows(2).all(|w| w[0] == w[1]),
                "a frame allocated more than the one before: {counts:?}"
            );
            (first, counts[0])
        };
        let (unregistered, allocations) = scene(game);
        register_special_model_types(vec!["test:unmatched".into()]);
        assert!(super::register(vec![(
            "test:unmatched".into(),
            mod_api::SpecialModelPolicy::Stack,
            Arc::new(Unmatched)
        )]));
        let (registered, registered_allocations) = scene(game);
        assert_eq!(registered, unregistered);
        assert_eq!(registered_allocations, allocations);
    });
}

/// Draws one stone cube for every stack.
struct Stone;
impl SpecialModelRenderer for Stone {
    fn extract(
        &self,
        _: &dyn ClientHost,
        _: &mod_api::ItemStack,
        _: &sunrise_core::nbt::Tag,
        _: &mod_api::SpecialModelContext,
    ) -> Result<Vec<SpecialModelPart>, String> {
        Ok(vec![SpecialModelPart::new("minecraft:block/stone")])
    }
}

/// Nanoseconds per resolve and submit of the stone cube, drawn three ways: stone's own
/// `minecraft:model`; a registered special drawing the same cube with its answer kept (an
/// unchanged stack, frame after frame); and a first extraction (the answers dropped before every
/// draw, so the renderer is asked, its answer checked, prepared and kept). Prints median, p95 and
/// p99 of 10,000 samples after 1,000 to warm up. It registers the process's renderers, so it
/// runs alone, by hand, with an optimized build:
/// `SUNRISE_TEST_RESOURCES=... cargo test --profile fast -p sunrise-client --lib -- --ignored
/// --nocapture special_item_draw_cost`.
#[test]
#[ignore = "measurement; needs SUNRISE_TEST_RESOURCES"]
fn special_item_draw_cost() {
    const WARM: usize = 1_000;
    const SAMPLES: usize = 10_000;
    register_special_model_types(vec!["test:stone".into()]);
    assert!(super::register(vec![(
        "test:stone".into(),
        mod_api::SpecialModelPolicy::Stack,
        Arc::new(Stone)
    )]));
    crate::game::native_entity_refs::tests::with_game(|game| {
        // Dirt becomes a special model drawing stone's cube under stone's display transforms.
        let special = ClientItem::from_json(
            r#"{"model":{"type":"minecraft:special","base":"minecraft:block/stone","model":{"type":"test:stone"}}}"#,
        )
        .unwrap();
        Arc::get_mut(&mut game.pack.item_renderer)
            .expect("the game's item renderer is shared")
            .models
            .definitions
            .insert("minecraft:dirt".into(), special);
        give_every_quad_a_sprite(game);
        let (stone, dirt) = (
            ItemStack::new("minecraft:stone", 1),
            ItemStack::new("minecraft:dirt", 1),
        );
        let mut out = Vec::new();
        let cube_vertices = {
            draw(game, &stone, &mut out);
            std::mem::take(&mut out).len()
        };
        draw(game, &dirt, &mut out);
        assert_eq!(out.len(), cube_vertices, "the special draws another shape");
        let mut measure = |what: &str, game: &Game<'_>, before: &dyn Fn(&Game<'_>), stack| {
            let mut times = Vec::with_capacity(SAMPLES);
            for sample in 0..WARM + SAMPLES {
                before(game);
                out.clear();
                let start = std::time::Instant::now();
                std::hint::black_box(draw(game, stack, &mut out));
                let elapsed = start.elapsed().as_nanos();
                if sample >= WARM {
                    times.push(elapsed);
                }
            }
            times.sort_unstable();
            println!(
                "SPECIAL_ITEM_COST draw={what} samples={SAMPLES} median_ns={} p95_ns={} p99_ns={}",
                times[SAMPLES / 2],
                times[SAMPLES * 95 / 100],
                times[SAMPLES * 99 / 100]
            );
        };
        measure("built_in_model", game, &|_| {}, &stone);
        measure(
            "special_kept",
            game,
            &|game| game.begin_frame_draws(),
            &dirt,
        );
        // Two frame starts drop every answer the last frame drew.
        measure(
            "special_first_extraction",
            game,
            &|game| {
                game.begin_frame_draws();
                game.begin_frame_draws();
            },
            &dirt,
        );
    });
}
